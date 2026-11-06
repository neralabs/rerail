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
//!   order over a thread per byte, a SIMD-group ballot for each mask.
//!
//! Operations are added one at a time, each through every executor before the next.

use super::Error;

/// An operation. Operands are earlier values.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(in crate::code_rule_dsl) enum BlockOp {
    /// The bytes equal to the byte: simdjson's `in.eq(c)`.
    Eq(u8),
}

impl BlockOp {
    /// The operation's text mnemonic and operands (`rir`).
    pub(super) fn text(&self) -> String {
        match self {
            BlockOp::Eq(byte) => format!("eq {byte:#04x}"),
        }
    }

    /// Parse `text`'s output: a mnemonic and its operands, as words.
    pub(super) fn parse(words: &[String]) -> Result<Self, String> {
        let byte = |word: &str| -> Result<u8, String> {
            word.strip_prefix("0x")
                .and_then(|hex| u8::from_str_radix(hex, 16).ok())
                .ok_or_else(|| format!("a bad byte `{word}`"))
        };
        match words {
            [name, b] if name == "eq" => Ok(BlockOp::Eq(byte(b)?)),
            _ => Err(format!("an unknown block operation `{}`", words.join(" "))),
        }
    }

    fn canonical(&self, out: &mut Vec<u8>) {
        match self {
            BlockOp::Eq(byte) => out.extend_from_slice(&[0, *byte]),
        }
    }
}

/// A block program: its operations and the values it outputs.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub(in crate::code_rule_dsl) struct BlockProgram {
    pub(super) ops: Vec<BlockOp>,
    pub(super) outputs: Vec<u16>,
}

impl BlockProgram {
    /// The verifier's rules for a block program: at least one operation and one output, and
    /// outputs that name values.
    pub(super) fn verify(&self) -> Result<(), Error> {
        let fits = self.ops.len() <= usize::from(u16::MAX);
        let outputs = !self.outputs.is_empty() && self.outputs.iter().all(|&v| usize::from(v) < self.ops.len());
        if self.ops.is_empty() || !fits || !outputs {
            return Err(Error::InvalidTarget(0));
        }
        Ok(())
    }

    /// The bytes the program digest covers.
    pub(super) fn canonical_bytes(&self) -> Vec<u8> {
        let mut out = Vec::new();
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
            let words: Vec<u64> = match *op {
                BlockOp::Eq(byte) => blocks.iter().map(|block| eq(block, byte)).collect(),
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
