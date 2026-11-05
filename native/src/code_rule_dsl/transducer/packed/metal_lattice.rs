//! File-local lattice FIX on Metal (list step 17; 02 §"Document-local lattice strata", 03 §7,
//! I10's R0): `logic::native::lattice_fixpoint`'s graph (seed values, copy edges, binary
//! transfers) over the flat lattice, solved on the device work queue (`metal_work_queue`). An
//! item is a cell to evaluate; a cell whose value rose pushes its dependents, which any idle
//! worker takes up, across every threadgroup, until none is active. The device, not the host,
//! decides each further evaluation (03 §7's device-side enqueue).
//!
//! The CPU's `lattice_fixpoint` is the reference: a monotone system over a finite lattice has one
//! least fixpoint, whatever order its cells are evaluated in.
use super::*;

const SOURCE: &str = include_str!("metal_lattice.metal");
/// The absent value (`logic::native::BOT`).
const BOT: i64 = i64::MIN;
/// The top value (`logic::native::TOP`).
const TOP: i64 = 1 << 62;

#[repr(C)]
#[derive(Clone, Copy)]
struct Args {
    cells: u32,
    capacity: u32,
    polls: u32,
    pad: u32,
}

/// A flat-lattice system: per cell its seed (`BOT` where none), its copy sources, and transfers
/// `(cell, op, left, right)` (`op` an ASCII byte: `+`, `-`, `*`, else TOP).
pub(in crate::code_rule_dsl) struct System<'a> {
    pub(in crate::code_rule_dsl) seeds: &'a [i64],
    /// (cell, source).
    pub(in crate::code_rule_dsl) copies: &'a [(u32, u32)],
    pub(in crate::code_rule_dsl) bins: &'a [(u32, u8, u32, u32)],
}

/// The solved values, the launches the queue took, the cells evaluated, and the evaluations
/// pushed again because another worker was publishing their cell.
pub(in crate::code_rule_dsl) struct Solved {
    pub(in crate::code_rule_dsl) values: Vec<i64>,
    pub(in crate::code_rule_dsl) launches: usize,
    pub(in crate::code_rule_dsl) visits: u64,
    pub(in crate::code_rule_dsl) retries: u64,
    /// The command buffers' GPU seconds, the host's wall seconds from its first buffer to the last
    /// completion, where they went (`profile::PHASES`), and the buffer bytes made and reused.
    pub(in crate::code_rule_dsl) gpu: f64,
    pub(in crate::code_rule_dsl) wall: f64,
    pub(in crate::code_rule_dsl) phases: [f64; 7],
    pub(in crate::code_rule_dsl) made: u64,
    pub(in crate::code_rule_dsl) reused: u64,
}

/// `(start, items)` CSR of `edges` by their first element over `n` cells.
fn csr(n: usize, mut edges: Vec<(u32, u32)>) -> (Vec<u32>, Vec<u32>) {
    edges.sort_unstable();
    let mut start = vec![0_u32; n + 1];
    for &(a, _) in &edges {
        start[a as usize + 1] += 1;
    }
    for at in 0..n {
        start[at + 1] += start[at];
    }
    (start, edges.into_iter().map(|(_, b)| b).collect())
}

/// The system's least fixpoint on the device.
pub(in crate::code_rule_dsl) fn solve(system: &System<'_>) -> Result<Solved, Error> {
    let n = system.seeds.len();
    if n == 0 {
        return Ok(Solved { values: Vec::new(), launches: 0, visits: 0, retries: 0, gpu: 0.0, wall: 0.0, phases: [0.0; 7], made: 0, reused: 0 });
    }
    if n > u32::MAX as usize / 4 {
        return Err(Error::BackendCapacity);
    }
    let device = METAL_DEVICE.as_ref().ok_or(Error::UnsupportedBackend)?;
    let (copy_start, copies) = csr(n, system.copies.to_vec());
    let (bin_start, bin_index) = csr(n, system.bins.iter().enumerate().map(|(at, bin)| (bin.0, at as u32)).collect());
    let bins = system.bins.iter().map(|&(_, op, left, right)| [u32::from(op), left, right, 0]).collect::<Vec<_>>();
    let mut outs = system.copies.iter().map(|&(cell, source)| (source, cell)).collect::<Vec<_>>();
    outs.extend(system.bins.iter().flat_map(|&(cell, _, left, right)| [(left, cell), (right, cell)]));
    let (out_start, outs) = csr(n, outs);
    // The seeds' states (BOT 0, a constant 2, TOP 3), and the first items: their dependents.
    let state = system.seeds.iter().map(|&seed| match seed {
        BOT => 0_u32,
        TOP => 3,
        _ => 2,
    });
    let state = state.collect::<Vec<_>>();
    let words = n.div_ceil(32);
    let mut queued = vec![0_u32; words];
    let mut initial = Vec::new();
    for cell in 0..n {
        if system.seeds[cell] != BOT {
            for &dependent in &outs[out_start[cell] as usize..out_start[cell + 1] as usize] {
                if queued[dependent as usize / 32] & 1 << (dependent % 32) == 0 {
                    queued[dependent as usize / 32] |= 1 << (dependent % 32);
                    initial.push(dependent);
                }
            }
        }
    }
    // Every push is a cell's input rising (at most twice per edge) or a retry; the slack covers
    // retries and the indices a taker gave up.
    let groups = super::work_queue::groups(n);
    let capacity = 4 * (n + 2 * outs.len()) + (groups * super::work_queue::WIDTH) as usize * 4 + 4096;
    use super::blocks::{upload, zeroed};
    let begun = super::metal_backend::launch::host_seconds();
    let queue = super::work_queue::Queue::new(capacity, &initial)?;
    let state = upload(&state)?;
    let values = upload(system.seeds)?;
    let seeds = upload(system.seeds)?;
    let (copy_start, copies, bin_start, bin_index) = (upload(&copy_start)?, upload(&copies)?, upload(&bin_start)?, upload(&bin_index)?);
    let bins = upload(&bins)?;
    let (out_start, outs, queued) = (upload(&out_start)?, upload(&outs)?, upload(&queued)?);
    let stats = zeroed::<u32>(4)?;
    let made_in = super::metal_backend::launch::host_seconds() - begun;
    let args = Args { cells: n as u32, capacity: queue.capacity, polls: super::work_queue::POLLS, pad: 0 };
    let pipeline = pipeline()?;
    let ran = super::work_queue::run(device, &pipeline, &queue, groups, "Rerail lattice FIX", |encoder| {
        for (index, buffer) in [
            &state, &values, &seeds, &copy_start, &copies, &bin_start, &bin_index, &bins, &out_start, &outs, &queued, &queue.slots, &queue.control, &stats,
        ]
        .iter()
        .enumerate()
        {
            encoder.set_buffer(index as u64, Some(buffer.buffer()), 0);
        }
        encoder.set_bytes(14, std::mem::size_of::<Args>() as u64, std::ptr::from_ref(&args).cast());
    })?;
    let (made, reused) = [&state, &values, &seeds, &copy_start, &copies, &bin_start, &bin_index, &bins, &out_start, &outs, &queued, &queue.slots, &queue.control, &stats]
        .iter()
        .fold((0u64, 0u64), |(m, r), b| if b.made { (m + b.byte_length() as u64, r) } else { (m, r + b.byte_length() as u64) });
    let (stats, state, raw) = (stats.read::<u32>(4), state.read::<u32>(n), values.read::<i64>(n));
    let values = state.iter().zip(raw).map(|(&state, value)| match state {
        0 => BOT,
        3 => TOP,
        _ => value,
    });
    if std::env::var_os("RERAIL_LAUNCHES").is_some() {
        eprintln!("lattice queue: {n} cells, {} initial, {groups} groups, {} launches, {} visits, {} retries, {} slots used", initial.len(), ran.launches, stats[0], stats[1], queue.used());
    }
    let mut phases = ran.phases;
    phases[0] = made_in;
    Ok(Solved {
        values: values.collect(),
        launches: ran.launches,
        visits: u64::from(stats[0]),
        retries: u64::from(stats[1]),
        gpu: ran.gpu,
        wall: ran.wall + made_in,
        phases,
        made,
        reused,
    })
}

fn pipeline() -> Result<metal::ComputePipelineState, Error> {
    static PIPELINE: std::sync::Mutex<Option<metal::ComputePipelineState>> = std::sync::Mutex::new(None);
    let mut cached = PIPELINE.lock().map_err(|_| Error::BackendFailure)?;
    if let Some(pipeline) = &*cached {
        return Ok(pipeline.clone());
    }
    let pipeline = pipeline_named(&format!("{}\n{SOURCE}", super::work_queue::SOURCE), "lattice_queue")?;
    *cached = Some(pipeline.clone());
    Ok(pipeline)
}
