//! Nearest ancestors in a set on Metal, in one dispatch (decisions 0038, 0040): the answer is a
//! pure function of the tree, so it needs no shared queue or fences. A cell is (set, node). Its
//! state starts at its node's parent, final where the parent is the answer, and each thread
//! takes its target's current state until its own is final (`metal_nearest.metal`). States other
//! threads have already moved up let a thread skip ahead. One command buffer, one dispatch and
//! one wait per call, whatever the depth. The CPU's one pass in preorder (`typing::file_local`)
//! is the reference.
use super::*;
pub use crate::code_rule_dsl::logic::device::nearest::{Buffers, Solved};

const SOURCE: &str = include_str!("metal_nearest.metal");
/// No ancestor.
pub(in crate::code_rule_dsl) const NONE: u32 = u32::MAX;
/// A state's bit for a final target (`metal_nearest.metal`).
const FINAL: u32 = 1 << 31;

#[repr(C)]
#[derive(Clone, Copy)]
struct Args {
    nodes: u32,
    cells: u32,
    pad: [u32; 2],
}

/// A buffer of either kind.
enum Held {
    Fresh(MappedBuffer),
    Block(super::blocks::Block),
}

impl Held {
    fn buffer(&self) -> &metal::BufferRef {
        match self {
            Held::Fresh(b) => &b.buffer,
            Held::Block(b) => b.buffer(),
        }
    }

    fn read(&self, count: usize) -> Vec<u32> {
        let host = match self {
            Held::Fresh(b) => b.host(),
            Held::Block(b) => b.host(),
        };
        // SAFETY: the buffer holds at least `count` u32; the device is done with it.
        unsafe { std::slice::from_raw_parts(host.cast::<u32>(), count).to_vec() }
    }
}

fn pipeline() -> Result<metal::ComputePipelineState, Error> {
    static PIPELINE: std::sync::Mutex<Option<metal::ComputePipelineState>> = std::sync::Mutex::new(None);
    let mut cached = PIPELINE.lock().map_err(|_| Error::BackendFailure)?;
    if let Some(pipeline) = &*cached {
        return Ok(pipeline.clone());
    }
    let pipeline = pipeline_named(SOURCE, "nearest_jump")?;
    *cached = Some(pipeline.clone());
    Ok(pipeline)
}

/// Per cell (`set * nodes + node`), its node's nearest strict ancestor whose kind is in the set:
/// `target` starts at the parent (`NONE` for a root) and `done` says whether that is the answer
/// (the parent is in the set, or there is none).
pub(in crate::code_rule_dsl) fn solve(nodes: usize, target: &[u32], done: &[u8]) -> Result<Solved, Error> {
    solve_with(nodes, target, done, Buffers::Blocks)
}

pub(in crate::code_rule_dsl) fn solve_with(nodes: usize, target: &[u32], done: &[u8], how: Buffers) -> Result<Solved, Error> {
    use super::metal_backend::launch::{command_times, host_seconds};
    let cells = target.len();
    if cells == 0 {
        return Ok(Solved { targets: Vec::new(), rounds: 0, launches: 0, gpu: 0.0, wall: 0.0, phases: [0.0; 7], made: 0, reused: 0 });
    }
    // Node indices leave bit 31 free.
    if cells >= u32::MAX as usize || nodes == 0 || nodes >= FINAL as usize || cells % nodes != 0 || done.len() != cells {
        return Err(Error::BackendCapacity);
    }
    let device = METAL_DEVICE.as_ref().ok_or(Error::UnsupportedBackend)?;
    let _scope = super::metal_backend::launch::LaunchScope::enter();
    let pipeline = pipeline()?;
    let mut phases = [0.0f64; 7];
    let (mut made, mut reused) = (0u64, 0u64);
    let begun = host_seconds();
    let state: Vec<u32> = target.iter().zip(done).map(|(&t, &d)| if t == NONE { NONE } else if d != 0 { t | FINAL } else { t }).collect();
    let (state, stats, queue) = match how {
        Buffers::Fresh => {
            let state = Held::Fresh(MappedBuffer::from_slice(device, &state)?);
            let stats = Held::Fresh(MappedBuffer::zeroed::<u32>(device, 1)?);
            made += (cells * 4 + 4) as u64;
            phases[0] += host_seconds() - begun;
            let at = host_seconds();
            let queue = rerail_metal::queue()?;
            phases[1] += host_seconds() - at;
            (state, stats, queue)
        }
        Buffers::Blocks => {
            use super::blocks;
            let blocks = [blocks::upload(&state)?, blocks::zeroed::<u32>(1)?];
            for b in &blocks {
                *if b.made { &mut made } else { &mut reused } += b.byte_length() as u64;
            }
            let [state, stats] = blocks;
            phases[0] += host_seconds() - begun;
            let at = host_seconds();
            let queue = blocks::queue(device)?;
            phases[1] += host_seconds() - at;
            (Held::Block(state), Held::Block(stats), queue)
        }
    };
    let at = host_seconds();
    let width = pipeline.thread_execution_width().max(1) * 4;
    let groups = (cells as u64).div_ceil(width);
    let command = queue.new_command_buffer();
    command.set_label("Rerail nearest");
    let encoder = command.new_compute_command_encoder();
    encoder.set_label("nearest");
    encoder.set_compute_pipeline_state(&pipeline);
    encoder.set_buffer(0, Some(state.buffer()), 0);
    encoder.set_buffer(1, Some(stats.buffer()), 0);
    let args = Args { nodes: nodes as u32, cells: cells as u32, pad: [0; 2] };
    encoder.set_bytes(2, std::mem::size_of::<Args>() as u64, std::ptr::from_ref(&args).cast());
    encoder.checked_dispatch_thread_groups(command, MTLSize::new(groups, 1, 1), MTLSize::new(width, 1, 1));
    encoder.end_encoding();
    let committed = host_seconds();
    phases[2] += committed - at;
    commit_checked(command)?;
    wait_checked(command)?;
    let woke = host_seconds();
    let [kernel, start, end] = command_times(command);
    phases[3] += (kernel - committed).max(0.0);
    phases[4] += (start - kernel).max(0.0);
    phases[5] += (end - start).max(0.0);
    phases[6] += (woke - end).max(0.0);
    let out = state.read(cells);
    if out.iter().any(|&v| v & FINAL == 0) {
        return Err(Error::BackendFailure);
    }
    let targets = out.into_iter().map(|v| if v == NONE { NONE } else { v & !FINAL }).collect();
    Ok(Solved { targets, rounds: stats.read(1)[0], launches: 1, gpu: end - start, wall: host_seconds() - begun, phases, made, reused })
}
