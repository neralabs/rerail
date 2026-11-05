//! Nearest ancestors in a set on Metal by pointer jumping (decision 0038), not on the work queue:
//! the answer is a pure function of the tree, so it needs no shared queue or fences. A cell is
//! (set, node); its target starts at the node's parent and jumps to its target's target until it
//! is final (`metal_nearest.metal`). All rounds are dispatches in one command buffer; where the
//! last round's flag shows a cell still open (a tree deeper than the rounds reach), more rounds
//! follow. The CPU's one pass in preorder (`typing::file_local`) is the reference.
use super::*;

const SOURCE: &str = include_str!("metal_nearest.metal");
/// No ancestor.
pub(in crate::code_rule_dsl) const NONE: u32 = u32::MAX;
/// Rounds encoded at a time: 2^6 = 64 levels, about what syntax trees reach; a deeper tree takes
/// another batch (a commit and a wait, about 0.3 ms).
const ROUNDS: u32 = 6;

#[repr(C)]
#[derive(Clone, Copy)]
struct Args {
    nodes: u32,
    cells: u32,
    round: u32,
    pad: u32,
}

/// The solved targets per cell (set-major), the rounds run, the command buffers' GPU seconds, the
/// host's wall seconds from the first buffer made to the last completion, and where those went
/// (`profile::PHASES`), with the buffer bytes made and reused.
pub(in crate::code_rule_dsl) struct Solved {
    pub(in crate::code_rule_dsl) targets: Vec<u32>,
    pub(in crate::code_rule_dsl) rounds: u32,
    pub(in crate::code_rule_dsl) launches: usize,
    pub(in crate::code_rule_dsl) gpu: f64,
    pub(in crate::code_rule_dsl) wall: f64,
    pub(in crate::code_rule_dsl) phases: [f64; 7],
    pub(in crate::code_rule_dsl) made: u64,
    pub(in crate::code_rule_dsl) reused: u64,
}

/// How a solve gets its buffers and queue: made for the call and dropped after (`Fresh`, kept
/// to measure against), or leased from the pre-pinned blocks with the kept queue (`Blocks`,
/// decision 0039).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(in crate::code_rule_dsl) enum Buffers {
    Fresh,
    Blocks,
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

    fn host(&self) -> *mut u8 {
        match self {
            Held::Fresh(b) => b.host(),
            Held::Block(b) => b.host(),
        }
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
    if cells >= u32::MAX as usize || nodes == 0 || cells % nodes != 0 {
        return Err(Error::BackendCapacity);
    }
    let device = METAL_DEVICE.as_ref().ok_or(Error::UnsupportedBackend)?;
    let _scope = super::metal_backend::launch::LaunchScope::enter();
    let pipeline = pipeline()?;
    let mut phases = [0.0f64; 7];
    let (mut made, mut reused) = (0u64, 0u64);
    let begun = host_seconds();
    let at = host_seconds();
    let (pairs, flags, queue) = match how {
        Buffers::Fresh => {
            let made_now = |b: MappedBuffer| Held::Fresh(b);
            let pairs = [
                (made_now(MappedBuffer::from_slice(device, target)?), made_now(MappedBuffer::from_slice(device, done)?)),
                (made_now(MappedBuffer::zeroed::<u32>(device, cells)?), made_now(MappedBuffer::zeroed::<u8>(device, cells)?)),
            ];
            let flags = Held::Fresh(MappedBuffer::zeroed::<u32>(device, ROUNDS as usize)?);
            made += (cells * 10 + ROUNDS as usize * 4) as u64;
            phases[0] += host_seconds() - at;
            let at = host_seconds();
            let queue = device.new_command_queue();
            phases[1] += host_seconds() - at;
            (pairs, flags, queue)
        }
        Buffers::Blocks => {
            use super::blocks;
            let blocks = [blocks::upload(target)?, blocks::upload(done)?, blocks::lease(cells * 4)?, blocks::lease(cells)?, blocks::lease(ROUNDS as usize * 4)?];
            for b in &blocks {
                *if b.made { &mut made } else { &mut reused } += b.byte_length() as u64;
            }
            let [t0, d0, t1, d1, f] = blocks;
            phases[0] += host_seconds() - at;
            let at = host_seconds();
            let queue = blocks::queue(device)?;
            phases[1] += host_seconds() - at;
            ([(Held::Block(t0), Held::Block(d0)), (Held::Block(t1), Held::Block(d1))], Held::Block(f), queue)
        }
    };
    let width = pipeline.thread_execution_width().max(1) * 4;
    let groups = (cells as u64).div_ceil(width);
    let (mut gpu, mut round, mut launches) = (0.0, 0u32, 0usize);
    loop {
        let at = host_seconds();
        // SAFETY: ROUNDS u32 flags; no command is using them.
        unsafe { std::ptr::write_bytes(flags.host(), 0, ROUNDS as usize * 4) };
        let command = queue.new_command_buffer();
        command.set_label("Rerail nearest jump");
        let encoder = command.new_compute_command_encoder();
        encoder.set_compute_pipeline_state(&pipeline);
        for r in 0..ROUNDS {
            let (from, to) = (&pairs[(round % 2) as usize], &pairs[((round + 1) % 2) as usize]);
            let args = Args { nodes: nodes as u32, cells: cells as u32, round: r, pad: 0 };
            for (index, buffer) in [&from.0, &from.1, &to.0, &to.1, &flags].iter().enumerate() {
                encoder.set_buffer(index as u64, Some(buffer.buffer()), 0);
            }
            encoder.set_bytes(5, std::mem::size_of::<Args>() as u64, std::ptr::from_ref(&args).cast());
            encoder.checked_dispatch_thread_groups(command, MTLSize::new(groups, 1, 1), MTLSize::new(width, 1, 1));
            round += 1;
            launches += 1;
        }
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
        gpu += end - start;
        let open = unsafe { *flags.host().cast::<u32>().add(ROUNDS as usize - 1) };
        if open == 0 {
            break;
        }
        if round >= 64 {
            return Err(Error::BackendFailure);
        }
    }
    let out = &pairs[(round % 2) as usize].0;
    let targets = unsafe { std::slice::from_raw_parts(out.host().cast::<u32>(), cells).to_vec() };
    Ok(Solved { targets, rounds: round, launches, gpu, wall: host_seconds() - begun, phases, made, reused })
}
