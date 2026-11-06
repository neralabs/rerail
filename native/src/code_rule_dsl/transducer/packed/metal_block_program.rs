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
    let words = dispatch(&pipeline, bytes, width, slots, |encoder, lanes| {
        let args = BlockArgs { ops: slots as u32, lanes, pad: [0; 2] };
        encoder.set_bytes(2, std::mem::size_of::<BlockArgs>() as u64, std::ptr::from_ref(&args).cast());
        let device = METAL_DEVICE.as_ref().ok_or(Error::UnsupportedBackend)?;
        let steps_buffer = MappedBuffer::from_slice(device, &steps)?;
        encoder.set_buffer(3, Some(&steps_buffer.buffer), 0);
        Ok(steps_buffer)
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

/// The native IR as one straight-line MSL kernel, `tempo_block`, for lanes of `width`.
pub(in crate::code_rule_dsl) fn generate(native: &BlockNative, width: LaneWidth) -> String {
    let m = width.word();
    let chunks = width.bytes() / 16;
    let bits = width.bytes();
    let mut out = format!(
        "#include <metal_stdlib>\nusing namespace metal;\n\n{EQ16}\n\
         kernel void tempo_block(\n\
         \x20   device const uint4* bytes [[buffer(0)]],\n\
         \x20   device {m}* out [[buffer(1)]],\n\
         \x20   constant uint& lanes [[buffer(2)]],\n\
         \x20   uint lane [[thread_position_in_grid]]) {{\n"
    );
    if native.insts.iter().any(|i| matches!(i, BlockInst::EqByte(_))) {
        for c in 0..chunks {
            out.push_str(&format!("  const uint4 c{c} = bytes[lane * {chunks}u + {c}u];\n"));
        }
    }
    for (at, inst) in native.insts.iter().enumerate() {
        let rhs = match *inst {
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
    let pipeline = pipeline_named(&generate(native, width), "tempo_block")?;
    let words = dispatch(&pipeline, bytes, width, native.outputs.len(), |encoder, lanes| {
        encoder.set_bytes(2, std::mem::size_of::<u32>() as u64, std::ptr::from_ref(&lanes).cast());
        Ok(())
    })?;
    Ok(words.iter().map(|w| finish(w, bytes.len())).collect())
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
