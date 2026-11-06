//! Block programs: the transducer's side program over source bytes
//! (`docs/parser-generators/12-what-is-wanted.md` §3), held in `Program::block_programs` as value
//! programs are held in `Program::value_programs`.
//!
//! A block program is a straight-line list of operations over 64-bit masks: bit `j` of a block's
//! word is byte `j` of the block's 64 source bytes. Operation `i` defines value `%i`. Every
//! executor lowers each operation to straight bit operations:
//!
//! - the CPU interpreter here: one dispatch per operation over the blocks, each block a NEON
//!   compare and a bitmask (a SWAR form off aarch64);
//! - the native IR (`native_ir::BlockNative`), which Cranelift (`native_cpu_block`) emits as one
//!   loop over blocks around a straight-line body, and Metal (`metal_block_program`) emits as a
//!   straight-line kernel;
//! - the Metal interpreter (`metal_block_program`), the set program's shape: every operation in
//!   order, each lane building its own mask word of 16, 32 or 64 bytes by SWAR on `uint4`s.
//!
//! Operations are added one at a time, each through every executor before the next.
//!
//! The block size is the executor's, not the program's: a program means one bit per source
//! byte. The CPU works in 64-byte words; Metal gives each lane 16, 32 or 64 bytes
//! (`metal_block_program::LaneWidth`).

use super::Error;

/// An operation. Operands are earlier values.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(in crate::code_rule_dsl) enum BlockOp {
    /// The bytes equal to the byte: simdjson's `in.eq(c)`.
    Eq(u8),
    /// The bytes in any class of `bits` of table `table` (`ClassTable`).
    Class { table: u16, bits: u8 },
    /// The same 64-bit pattern over every 64 bytes of the stream, e.g. simdjson's `ODD_BITS`.
    Const(u64),
    Not(u16),
    And(u16, u16),
    Or(u16, u16),
    Xor(u16, u16),
    /// `a & !b`.
    AndNot(u16, u16),
}

impl BlockOp {
    /// The operation's text mnemonic and operands (`rir`).
    pub(super) fn text(&self) -> String {
        let two = |name: &str, a: &u16, b: &u16| format!("{name} %{a}, %{b}");
        match self {
            BlockOp::Eq(byte) => format!("eq {byte:#04x}"),
            BlockOp::Class { table, bits } => format!("class t{table} {bits:#04x}"),
            BlockOp::Const(c) => format!("const {c:#018x}"),
            BlockOp::Not(a) => format!("not %{a}"),
            BlockOp::And(a, b) => two("and", a, b),
            BlockOp::Or(a, b) => two("or", a, b),
            BlockOp::Xor(a, b) => two("xor", a, b),
            BlockOp::AndNot(a, b) => two("andnot", a, b),
        }
    }

    /// The values the operation reads.
    pub(super) fn operands(&self) -> Vec<u16> {
        match *self {
            BlockOp::Eq(_) | BlockOp::Class { .. } | BlockOp::Const(_) => vec![],
            BlockOp::Not(a) => vec![a],
            BlockOp::And(a, b) | BlockOp::Or(a, b) | BlockOp::Xor(a, b) | BlockOp::AndNot(a, b) => vec![a, b],
        }
    }

    /// Parse `text`'s output: a mnemonic and its operands, as words.
    pub(super) fn parse(words: &[String]) -> Result<Self, String> {
        let byte = |word: &str| -> Result<u8, String> {
            word.strip_prefix("0x")
                .and_then(|hex| u8::from_str_radix(hex, 16).ok())
                .ok_or_else(|| format!("a bad byte `{word}`"))
        };
        let value = |word: &str| -> Result<u16, String> {
            word.trim_end_matches(',')
                .strip_prefix('%')
                .and_then(|v| v.parse().ok())
                .ok_or_else(|| format!("a bad value `{word}`"))
        };
        let words: Vec<&str> = words.iter().map(String::as_str).collect();
        match words.as_slice() {
            ["eq", b] => Ok(BlockOp::Eq(byte(b)?)),
            ["class", t, b] => Ok(BlockOp::Class {
                table: t.strip_prefix('t').and_then(|t| t.parse().ok()).ok_or_else(|| format!("a bad table `{t}`"))?,
                bits: byte(b)?,
            }),
            ["const", c] => c
                .strip_prefix("0x")
                .and_then(|hex| u64::from_str_radix(hex, 16).ok())
                .map(BlockOp::Const)
                .ok_or_else(|| format!("a bad constant `{c}`")),
            ["not", a] => Ok(BlockOp::Not(value(a)?)),
            ["and", a, b] => Ok(BlockOp::And(value(a)?, value(b)?)),
            ["or", a, b] => Ok(BlockOp::Or(value(a)?, value(b)?)),
            ["xor", a, b] => Ok(BlockOp::Xor(value(a)?, value(b)?)),
            ["andnot", a, b] => Ok(BlockOp::AndNot(value(a)?, value(b)?)),
            _ => Err(format!("an unknown block operation `{}`", words.join(" "))),
        }
    }

    fn canonical(&self, out: &mut Vec<u8>) {
        let two = |out: &mut Vec<u8>, tag: u8, a: u16, b: u16| {
            out.push(tag);
            out.extend_from_slice(&a.to_le_bytes());
            out.extend_from_slice(&b.to_le_bytes());
        };
        match *self {
            BlockOp::Eq(byte) => out.extend_from_slice(&[0, byte]),
            BlockOp::Class { table, bits } => {
                out.push(7);
                out.extend_from_slice(&table.to_le_bytes());
                out.push(bits);
            }
            BlockOp::Const(c) => {
                out.push(1);
                out.extend_from_slice(&c.to_le_bytes());
            }
            BlockOp::Not(a) => two(out, 2, a, a),
            BlockOp::And(a, b) => two(out, 3, a, b),
            BlockOp::Or(a, b) => two(out, 4, a, b),
            BlockOp::Xor(a, b) => two(out, 5, a, b),
            BlockOp::AndNot(a, b) => two(out, 6, a, b),
        }
    }
}

/// A nibble table pair for `class` (`03-bitlex.md` §3.1, simdjson's `classify`): byte `b` is in
/// class bit `c` when bit `c` is set in both `lo[b & 15]` and `hi[b >> 4]`. Up to eight classes
/// per pair, each a product of a set of high nibbles and a set of low nibbles.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub(in crate::code_rule_dsl) struct ClassTable {
    pub(super) lo: [u8; 16],
    pub(super) hi: [u8; 16],
}

impl ClassTable {
    /// Whether byte `b` is in any class of `bits`.
    pub(super) fn member(&self, b: u8, bits: u8) -> bool {
        self.lo[usize::from(b & 15)] & self.hi[usize::from(b >> 4)] & bits != 0
    }
}

/// A block program: its class tables, its operations and the values it outputs.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub(in crate::code_rule_dsl) struct BlockProgram {
    pub(super) tables: Vec<ClassTable>,
    pub(super) ops: Vec<BlockOp>,
    pub(super) outputs: Vec<u16>,
}

impl BlockProgram {
    /// The verifier's rules for a block program: at least one operation and one output, every
    /// operand an earlier value, and outputs that name values.
    pub(super) fn verify(&self) -> Result<(), Error> {
        let fits = self.ops.len() <= usize::from(u16::MAX);
        let outputs = !self.outputs.is_empty() && self.outputs.iter().all(|&v| usize::from(v) < self.ops.len());
        let earlier = self
            .ops
            .iter()
            .enumerate()
            .all(|(at, op)| op.operands().iter().all(|&v| usize::from(v) < at));
        let tables = self.ops.iter().all(|op| match *op {
            BlockOp::Class { table, bits } => usize::from(table) < self.tables.len() && bits != 0,
            _ => true,
        });
        if self.ops.is_empty() || !fits || !outputs || !earlier || !tables {
            return Err(Error::InvalidTarget(0));
        }
        Ok(())
    }

    /// The bytes the program digest covers.
    pub(super) fn canonical_bytes(&self) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&(self.tables.len() as u32).to_le_bytes());
        for table in &self.tables {
            out.extend_from_slice(&table.lo);
            out.extend_from_slice(&table.hi);
        }
        out.extend_from_slice(&(self.ops.len() as u32).to_le_bytes());
        for op in &self.ops {
            op.canonical(&mut out);
        }
        out.extend_from_slice(&(self.outputs.len() as u32).to_le_bytes());
        for v in &self.outputs {
            out.extend_from_slice(&v.to_le_bytes());
        }
        out
    }

    /// The CPU interpreter: every output's words over `bytes`, one dispatch per operation over
    /// all the blocks. Bits past the end of `bytes` are zero.
    pub(super) fn interpret(&self, bytes: &[u8]) -> Vec<Vec<u64>> {
        let blocks = padded(bytes);
        let mut values: Vec<Vec<u64>> = Vec::with_capacity(self.ops.len());
        for op in &self.ops {
            let v = |at: u16| &values[usize::from(at)];
            let two = |a: u16, b: u16, f: fn(u64, u64) -> u64| -> Vec<u64> {
                v(a).iter().zip(v(b)).map(|(&x, &y)| f(x, y)).collect()
            };
            let words: Vec<u64> = match *op {
                BlockOp::Eq(byte) => blocks.iter().map(|block| eq(block, byte)).collect(),
                BlockOp::Class { table, bits } => {
                    let table = &self.tables[usize::from(table)];
                    blocks.iter().map(|block| class(block, table, bits)).collect()
                }
                BlockOp::Const(c) => vec![c; blocks.len()],
                BlockOp::Not(a) => v(a).iter().map(|&x| !x).collect(),
                BlockOp::And(a, b) => two(a, b, |x, y| x & y),
                BlockOp::Or(a, b) => two(a, b, |x, y| x | y),
                BlockOp::Xor(a, b) => two(a, b, |x, y| x ^ y),
                BlockOp::AndNot(a, b) => two(a, b, |x, y| x & !y),
            };
            values.push(words);
        }
        self.outputs
            .iter()
            .map(|&v| {
                let mut words = values[usize::from(v)].clone();
                mask_tail(&mut words, bytes.len());
                words
            })
            .collect()
    }
}

/// The source as whole 64-byte blocks, zero past its end.
pub(super) fn padded(bytes: &[u8]) -> Vec<[u8; 64]> {
    bytes
        .chunks(64)
        .map(|chunk| {
            let mut block = [0u8; 64];
            block[..chunk.len()].copy_from_slice(chunk);
            block
        })
        .collect()
}

/// Clear the bits past `len` bytes.
pub(super) fn mask_tail(words: &mut [u64], len: usize) {
    if let Some(last) = words.last_mut()
        && len % 64 != 0
    {
        *last &= (1u64 << (len % 64)) - 1;
    }
}

/// `eq` over one block: four 16-byte compares and simdjson's arm64 `to_bitmask`.
#[cfg(target_arch = "aarch64")]
pub(super) fn eq(block: &[u8; 64], byte: u8) -> u64 {
    use core::arch::aarch64::*;
    // SAFETY: NEON is in the aarch64 baseline; each load reads 16 bytes of the 64-byte block.
    unsafe {
        let c = vdupq_n_u8(byte);
        let at = |offset: usize| vceqq_u8(vld1q_u8(block.as_ptr().add(offset)), c);
        to_bitmask(at(0), at(16), at(32), at(48))
    }
}

/// Off aarch64, the SWAR form.
#[cfg(not(target_arch = "aarch64"))]
pub(super) fn eq(block: &[u8; 64], byte: u8) -> u64 {
    eq_swar(block, byte)
}

/// `class` over one block: two nibble lookups per 16 bytes (`vqtbl1q_u8`, as simdjson's arm64
/// `classify`), an AND, a test against the class bits, and the bitmask.
#[cfg(target_arch = "aarch64")]
pub(super) fn class(block: &[u8; 64], table: &ClassTable, bits: u8) -> u64 {
    use core::arch::aarch64::*;
    // SAFETY: NEON is in the aarch64 baseline; each load reads 16 bytes of the block or a table.
    unsafe {
        let lo = vld1q_u8(table.lo.as_ptr());
        let hi = vld1q_u8(table.hi.as_ptr());
        let nibble = vdupq_n_u8(0x0f);
        let sel = vdupq_n_u8(bits);
        let at = |offset: usize| {
            let d = vld1q_u8(block.as_ptr().add(offset));
            let l = vqtbl1q_u8(lo, vandq_u8(d, nibble));
            let h = vqtbl1q_u8(hi, vshrq_n_u8::<4>(d));
            vtstq_u8(vandq_u8(l, h), sel)
        };
        to_bitmask(at(0), at(16), at(32), at(48))
    }
}

/// Off aarch64, the same lookups by SWAR are not available; the portable form reads the tables
/// a byte at a time. It is not a measured path (x86 would use `pshufb`).
#[cfg(not(target_arch = "aarch64"))]
pub(super) fn class(block: &[u8; 64], table: &ClassTable, bits: u8) -> u64 {
    block.iter().enumerate().fold(0, |m, (j, &b)| m | (u64::from(table.member(b, bits)) << j))
}

/// simdjson's arm64 `simd8x64<bool>::to_bitmask`: each byte weighted by its bit, then three
/// rounds of pairwise adds.
#[cfg(target_arch = "aarch64")]
unsafe fn to_bitmask(
    m0: core::arch::aarch64::uint8x16_t,
    m1: core::arch::aarch64::uint8x16_t,
    m2: core::arch::aarch64::uint8x16_t,
    m3: core::arch::aarch64::uint8x16_t,
) -> u64 {
    use core::arch::aarch64::*;
    const WEIGHTS: [u8; 16] = [1, 2, 4, 8, 16, 32, 64, 128, 1, 2, 4, 8, 16, 32, 64, 128];
    // SAFETY: NEON is in the aarch64 baseline; the load reads the 16-byte constant.
    unsafe {
        let w = vld1q_u8(WEIGHTS.as_ptr());
        let s0 = vpaddq_u8(vandq_u8(m0, w), vandq_u8(m1, w));
        let s1 = vpaddq_u8(vandq_u8(m2, w), vandq_u8(m3, w));
        let s = vpaddq_u8(s0, s1);
        let s = vpaddq_u8(s, s);
        vgetq_lane_u64::<0>(vreinterpretq_u64_u8(s))
    }
}

/// `eq` over one block by SWAR: eight 8-byte words, each a zero-byte test and a multiply that
/// gathers the eight high bits.
pub(super) fn eq_swar(block: &[u8; 64], byte: u8) -> u64 {
    const LOW: u64 = 0x7f7f_7f7f_7f7f_7f7f;
    const HIGH: u64 = 0x8080_8080_8080_8080;
    const GATHER: u64 = 0x0002_0408_1020_4081;
    let c = u64::from(byte) * 0x0101_0101_0101_0101;
    let word = |at: usize| {
        let w = u64::from_le_bytes(block[at * 8..at * 8 + 8].try_into().unwrap()) ^ c;
        // High bit of each byte that is zero, exactly (no borrow between bytes).
        let zero = !(((w & LOW) + LOW) | w) & HIGH;
        // Byte i's high bit (bit 8i + 7) to bit i.
        ((zero >> 7).wrapping_mul(GATHER) >> 49) & 0xff
    };
    word(0)
        | word(1) << 8
        | word(2) << 16
        | word(3) << 24
        | word(4) << 32
        | word(5) << 40
        | word(6) << 48
        | word(7) << 56
}
