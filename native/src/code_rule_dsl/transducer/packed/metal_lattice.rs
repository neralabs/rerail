//! File-local lattice FIX waves on Metal (list step 17; 02 §"Document-local lattice strata", 03
//! §7, I10's R0): `logic::native::lattice_fixpoint`'s graph (seed values, copy edges, binary
//! transfers) over the flat lattice, solved in one dispatch. Its cells split into components no
//! edge leaves (a file's lattice is one or more), each a threadgroup that runs its own semi-naive
//! waves until one changes nothing: the device, not the host, decides each further wave (03 §7's
//! device-side enqueue, inside the dispatch). The flat lattice has height 3, so a cell changes at
//! most twice and a component settles within `2 · cells + 2` waves.
//!
//! The CPU's `lattice_fixpoint` is the reference: a monotone system over a finite lattice has one
//! least fixpoint, whatever order its cells are evaluated in.
use super::*;
use metal::MTLSize;

const SOURCE: &str = include_str!("metal_lattice.metal");
/// A component's threads.
const WIDTH: u64 = 256;
/// The absent value (`logic::native::BOT`).
const BOT: i64 = i64::MIN;

#[repr(C)]
#[derive(Clone, Copy)]
struct Args {
    cells: u32,
    components: u32,
    max_waves: u32,
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

/// The solved values, the most waves a component ran, and the cells evaluated.
pub(in crate::code_rule_dsl) struct Solved {
    pub(in crate::code_rule_dsl) values: Vec<i64>,
    pub(in crate::code_rule_dsl) waves: u32,
    pub(in crate::code_rule_dsl) visits: u64,
    pub(in crate::code_rule_dsl) components: usize,
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

/// The cells grouped by component (union-find over the edges): the cells in component order, and
/// each component's range in it.
fn components(n: usize, system: &System<'_>) -> (Vec<u32>, Vec<u32>) {
    let mut parent = (0..n as u32).collect::<Vec<_>>();
    fn find(parent: &mut [u32], mut cell: u32) -> u32 {
        while parent[cell as usize] != cell {
            parent[cell as usize] = parent[parent[cell as usize] as usize];
            cell = parent[cell as usize];
        }
        cell
    }
    let mut unite = |a: u32, b: u32| {
        let (a, b) = (find(&mut parent, a), find(&mut parent, b));
        if a != b {
            parent[a.max(b) as usize] = a.min(b);
        }
    };
    for &(cell, source) in system.copies {
        unite(cell, source);
    }
    for &(cell, _, left, right) in system.bins {
        unite(cell, left);
        unite(cell, right);
    }
    let roots = (0..n as u32).map(|cell| find(&mut parent, cell)).collect::<Vec<_>>();
    let mut order = (0..n as u32).collect::<Vec<_>>();
    order.sort_by_key(|&cell| (roots[cell as usize], cell));
    let mut ranges = vec![0_u32];
    for at in 1..n {
        if roots[order[at] as usize] != roots[order[at - 1] as usize] {
            ranges.push(at as u32);
        }
    }
    ranges.push(n as u32);
    if n == 0 {
        ranges.truncate(1);
    }
    (order, ranges)
}

/// The system's least fixpoint on the device.
pub(in crate::code_rule_dsl) fn solve(system: &System<'_>) -> Result<Solved, Error> {
    let n = system.seeds.len();
    if n == 0 {
        return Ok(Solved { values: Vec::new(), waves: 0, visits: 0, components: 0 });
    }
    if n > u32::MAX as usize / 2 - 2 {
        return Err(Error::BackendCapacity);
    }
    let device = METAL_DEVICE.as_ref().ok_or(Error::UnsupportedBackend)?;
    let _scope = super::metal_backend::launch::LaunchScope::enter();
    let (copy_start, copies) = csr(n, system.copies.to_vec());
    let (bin_start, bin_index) = csr(n, system.bins.iter().enumerate().map(|(at, bin)| (bin.0, at as u32)).collect());
    let bins = system.bins.iter().map(|&(_, op, left, right)| [u32::from(op), left, right, 0]).collect::<Vec<_>>();
    let mut outs = system.copies.iter().map(|&(cell, source)| (source, cell)).collect::<Vec<_>>();
    outs.extend(system.bins.iter().flat_map(|&(cell, _, left, right)| [(left, cell), (right, cell)]));
    let (out_start, outs) = csr(n, outs);
    let (order, ranges) = components(n, system);
    // The first wave: the dependents of the seeded cells.
    let words = n.div_ceil(32);
    let mut dirty = vec![0_u32; words];
    for cell in 0..n {
        if system.seeds[cell] != BOT {
            for &dependent in &outs[out_start[cell] as usize..out_start[cell + 1] as usize] {
                dirty[dependent as usize / 32] |= 1 << (dependent % 32);
            }
        }
    }
    let upload = |data: &[u32]| MappedBuffer::from_slice(device, if data.is_empty() { &[0_u32][..] } else { data });
    let values = MappedBuffer::from_slice(device, system.seeds)?;
    let seeds = MappedBuffer::from_slice(device, system.seeds)?;
    let next = MappedBuffer::zeroed::<i64>(device, n)?;
    let (copy_start, copies, bin_start, bin_index) = (upload(&copy_start)?, upload(&copies)?, upload(&bin_start)?, upload(&bin_index)?);
    let bins = MappedBuffer::from_slice(device, if bins.is_empty() { &[[0_u32; 4]][..] } else { &bins })?;
    let (out_start, outs, order_buffer, ranges_buffer) = (upload(&out_start)?, upload(&outs)?, upload(&order)?, upload(&ranges)?);
    let dirty = upload(&dirty)?;
    let pending = MappedBuffer::zeroed::<u32>(device, words.max(1))?;
    let stats = MappedBuffer::zeroed::<u32>(device, 4)?;
    let components = ranges.len() - 1;
    let args = Args { cells: n as u32, components: components as u32, max_waves: 2 * n as u32 + 2, pad: 0 };
    let pipeline = pipeline()?;
    let queue = device.new_command_queue();
    let command = queue.new_command_buffer();
    command.set_label("Rerail lattice FIX");
    let encoder = command.new_compute_command_encoder();
    encoder.set_label("lattice_flat");
    encoder.set_compute_pipeline_state(&pipeline);
    for (index, buffer) in [
        &values, &seeds, &copy_start, &copies, &bin_start, &bin_index, &bins, &out_start, &outs, &order_buffer, &ranges_buffer, &dirty, &pending, &next, &stats,
    ]
    .iter()
    .enumerate()
    {
        encoder.set_buffer(index as u64, Some(&buffer.buffer), 0);
    }
    encoder.set_bytes(15, std::mem::size_of::<Args>() as u64, std::ptr::from_ref(&args).cast());
    encoder.checked_dispatch_thread_groups(command, MTLSize::new(components as u64, 1, 1), MTLSize::new(WIDTH, 1, 1));
    encoder.end_encoding();
    commit_checked(command)?;
    wait_checked(command)?;
    let read = |buffer: &MappedBuffer, count: usize| unsafe { std::slice::from_raw_parts(buffer.buffer.contents().cast::<u32>(), count).to_vec() };
    let stats = read(&stats, 4);
    if stats[2] != 0 {
        // A component past its wave bound: not a finite flat system.
        return Err(Error::BackendFailure);
    }
    let values = unsafe { std::slice::from_raw_parts(values.buffer.contents().cast::<i64>(), n).to_vec() };
    Ok(Solved { values, waves: stats[0], visits: u64::from(stats[1]), components })
}

fn pipeline() -> Result<metal::ComputePipelineState, Error> {
    static PIPELINE: std::sync::Mutex<Option<metal::ComputePipelineState>> = std::sync::Mutex::new(None);
    let mut cached = PIPELINE.lock().map_err(|_| Error::BackendFailure)?;
    if let Some(pipeline) = &*cached {
        return Ok(pipeline.clone());
    }
    let pipeline = pipeline_named(SOURCE, "lattice_flat")?;
    *cached = Some(pipeline.clone());
    Ok(pipeline)
}
