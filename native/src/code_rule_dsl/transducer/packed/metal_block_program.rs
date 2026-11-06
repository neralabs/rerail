//! Block programs on Metal (`block`), both ways the transducer runs on the device:
//!
//! - **the interpreter** (`metal_block_program.metal`): the set program's shape, one dispatch
//!   running every operation in order; a thread per source byte, a threadgroup per 64-byte block,
//!   and each mask word built by a SIMD-group ballot;
//! - **generated MSL** from the native IR (`BlockNative`), the same thread layout with the
//!   operations written out as one straight-line kernel: no loop and no switch.
//!
//! Both need 32-lane SIMD groups (two per block), which every Apple GPU has; anything else is
//! refused, not run on the CPU.
use super::*;
use crate::code_rule_dsl::transducer::packed::block::{BlockOp, BlockProgram, mask_tail, padded};
use crate::code_rule_dsl::transducer::packed::native_ir::{BlockInst, BlockNative};
use metal::MTLSize;

const SOURCE: &str = include_str!("metal_block_program.metal");
/// A block's threads: a byte each.
const WIDTH: u64 = 64;

#[repr(C)]
#[derive(Clone, Copy)]
struct BlockArgs {
    ops: u32,
    halves: u32,
    pad: [u32; 2],
}

fn encoded(op: &BlockOp) -> [u32; 4] {
    match *op {
        BlockOp::Eq(byte) => [0, 0, 0, u32::from(byte)],
    }
}

/// The interpreter: every output's words over `bytes`.
pub(in crate::code_rule_dsl) fn interpret(program: &BlockProgram, bytes: &[u8]) -> Result<Vec<Vec<u64>>, Error> {
    let steps: Vec<[u32; 4]> = program.ops.iter().map(encoded).collect();
    let pipeline = pipeline_named(SOURCE, "block_program")?;
    let slots = program.ops.len();
    let words = dispatch(&pipeline, bytes, slots, |encoder, halves| {
        let args = BlockArgs { ops: slots as u32, halves, pad: [0; 2] };
        encoder.set_bytes(2, std::mem::size_of::<BlockArgs>() as u64, std::ptr::from_ref(&args).cast());
        let steps_buffer = MappedBuffer::from_slice(METAL_DEVICE.as_ref().ok_or(Error::UnsupportedBackend)?, &steps)?;
        encoder.set_buffer(3, Some(&steps_buffer.buffer), 0);
        Ok(steps_buffer)
    })?;
    Ok(program.outputs.iter().map(|&v| finish(&words[usize::from(v)], bytes.len())).collect())
}

/// The native IR as one straight-line MSL kernel, `tempo_block`.
pub(in crate::code_rule_dsl) fn generate(native: &BlockNative) -> String {
    let mut out = String::from(
        "#include <metal_stdlib>\nusing namespace metal;\n\n\
         kernel void tempo_block(\n\
         \x20   device const uchar* bytes [[buffer(0)]],\n\
         \x20   device uint* out [[buffer(1)]],\n\
         \x20   constant uint& halves [[buffer(2)]],\n\
         \x20   uint byte [[thread_position_in_grid]],\n\
         \x20   uint lane [[thread_index_in_simdgroup]]) {\n\
         \x20 const uchar b = bytes[byte];\n",
    );
    for (at, inst) in native.insts.iter().enumerate() {
        match *inst {
            BlockInst::EqByte(byte) => {
                let _ = writeln!(
                    out,
                    "  const uint v{at} = static_cast<uint>(static_cast<simd_vote::vote_t>(simd_ballot(b == uchar({byte:#04x}))));"
                );
            }
        }
    }
    out.push_str("  if (lane == 0u) {\n");
    for (k, v) in native.outputs.iter().enumerate() {
        let _ = writeln!(out, "    out[{k}u * halves + byte / 32u] = v{v};");
    }
    out.push_str("  }\n}\n");
    out
}

/// Generated MSL run: every output's words over `bytes`.
pub(in crate::code_rule_dsl) fn run_generated(native: &BlockNative, bytes: &[u8]) -> Result<Vec<Vec<u64>>, Error> {
    let pipeline = pipeline_named(&generate(native), "tempo_block")?;
    let words = dispatch(&pipeline, bytes, native.outputs.len(), |encoder, halves| {
        encoder.set_bytes(2, std::mem::size_of::<u32>() as u64, std::ptr::from_ref(&halves).cast());
        Ok(())
    })?;
    Ok(words.iter().map(|w| finish(w, bytes.len())).collect())
}

/// One dispatch over `bytes`' blocks with `slots` slots of mask words; returns each slot's words.
fn dispatch<T>(
    pipeline: &metal::ComputePipelineState,
    bytes: &[u8],
    slots: usize,
    bind: impl FnOnce(&metal::ComputeCommandEncoderRef, u32) -> Result<T, Error>,
) -> Result<Vec<Vec<u64>>, Error> {
    let device = METAL_DEVICE.as_ref().ok_or(Error::UnsupportedBackend)?;
    if pipeline.thread_execution_width() != 32 {
        return Err(Error::UnsupportedBackend);
    }
    let _scope = super::metal_backend::launch::LaunchScope::enter();
    let blocks = padded(bytes);
    let flat: Vec<u8> = blocks.iter().flatten().copied().collect();
    let halves = (blocks.len() * 2) as u32;
    let source = MappedBuffer::from_slice(device, if flat.is_empty() { &[0u8][..] } else { &flat })?;
    let values = MappedBuffer::zeroed::<u32>(device, slots.max(1) * halves.max(1) as usize)?;
    if !blocks.is_empty() {
        let queue = device.new_command_queue();
        let command = queue.new_command_buffer();
        command.set_label("Rerail block program");
        let encoder = command.new_compute_command_encoder();
        encoder.set_label("block_program");
        encoder.set_compute_pipeline_state(pipeline);
        encoder.set_buffer(0, Some(&source.buffer), 0);
        encoder.set_buffer(1, Some(&values.buffer), 0);
        let _kept = bind(encoder, halves)?;
        encoder.checked_dispatch_thread_groups(command, MTLSize::new(blocks.len() as u64, 1, 1), MTLSize::new(WIDTH, 1, 1));
        encoder.end_encoding();
        commit_checked(command)?;
        wait_checked(command)?;
    }
    let base = values.buffer.contents().cast::<u32>();
    Ok((0..slots)
        .map(|slot| {
            // SAFETY: the buffer holds `slots * halves` halves, written by the kernel.
            let halves = unsafe { std::slice::from_raw_parts(base.add(slot * halves as usize), halves as usize) };
            halves.chunks(2).map(|h| u64::from(h[0]) | u64::from(h[1]) << 32).collect()
        })
        .collect())
}

fn finish(words: &[u64], len: usize) -> Vec<u64> {
    let mut words = words.to_vec();
    mask_tail(&mut words, len);
    words
}
