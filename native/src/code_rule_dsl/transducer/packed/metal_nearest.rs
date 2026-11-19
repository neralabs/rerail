//! Nearest ancestors in a set on the device, in one dispatch (decisions 0038, 0040): the answer is a pure function
//! of the tree, so it needs no shared queue or fences. A cell is (set, node). Its state starts at its node's parent,
//! final where the parent is the answer, and each thread takes its target's current state until its own is final
//! (`nearest.ukl`). States other threads have already moved up let a thread skip ahead. One command, one dispatch
//! and one wait per call, whatever the depth, on the process's device (decision 0440 step C). The CPU's one pass in
//! preorder (`typing::file_local`) is the reference.
use super::*;
pub use crate::code_rule_dsl::logic::device::nearest::{Buffers, Solved};
use rerail_metal::lane::{self, Buffer};

const SOURCE: &str = include_str!("nearest.ukl");
/// No ancestor.
pub(in crate::code_rule_dsl) const NONE: u32 = u32::MAX;
/// A state's bit for a final target (`nearest.ukl`).
const FINAL: u32 = 1 << 31;

#[repr(C)]
#[derive(Clone, Copy)]
struct Args {
    nodes: u32,
    cells: u32,
    pad: [u32; 2],
}
unsafe impl lane::Pod for Args {}

/// A buffer of either kind.
enum Held {
    Fresh(Buffer),
    Block(super::blocks::Block),
}

impl Held {
    fn buffer(&self) -> &Buffer {
        match self {
            Held::Fresh(b) => b,
            Held::Block(b) => b.device_buffer(),
        }
    }

    fn read(&self, count: usize) -> Vec<u32> {
        // SAFETY: the buffer holds at least `count` u32 in the host's memory; the device is done with it.
        unsafe { std::slice::from_raw_parts(self.buffer().contents().cast::<u32>(), count).to_vec() }
    }
}

/// Per cell (`set * nodes + node`), its node's nearest strict ancestor whose kind is in the set:
/// `target` starts at the parent (`NONE` for a root) and `done` says whether that is the answer
/// (the parent is in the set, or there is none).
pub(in crate::code_rule_dsl) fn solve(nodes: usize, target: &[u32], done: &[u8]) -> Result<Solved, Error> {
    solve_with(nodes, target, done, Buffers::Blocks)
}

pub(in crate::code_rule_dsl) fn solve_with(nodes: usize, target: &[u32], done: &[u8], how: Buffers) -> Result<Solved, Error> {
    use rerail_metal::launch::host_seconds;
    let cells = target.len();
    if cells == 0 {
        return Ok(Solved { targets: Vec::new(), rounds: 0, launches: 0, gpu: 0.0, wall: 0.0, phases: [0.0; 7], made: 0, reused: 0 });
    }
    // Node indices leave bit 31 free.
    if cells >= u32::MAX as usize || nodes == 0 || nodes >= FINAL as usize || cells % nodes != 0 || done.len() != cells {
        return Err(Error::BackendCapacity);
    }
    let device = lane::device()?;
    let _scope = rerail_metal::launch::LaunchScope::enter();
    let kernel = lane::kernel_fixed(SOURCE, "nearest_jump", None);
    let mut phases = [0.0f64; 7];
    let (mut made, mut reused) = (0u64, 0u64);
    let begun = host_seconds();
    let state: Vec<u32> = target.iter().zip(done).map(|(&t, &d)| if t == NONE { NONE } else if d != 0 { t | FINAL } else { t }).collect();
    let (state, stats) = match how {
        Buffers::Fresh => {
            made += (cells * 4 + 4) as u64;
            (Held::Fresh(lane::buffer_of(device, &state)), Held::Fresh(lane::buffer_of(device, &[0u32])))
        }
        Buffers::Blocks => {
            use super::blocks;
            let blocks = [blocks::upload(&state)?, blocks::zeroed::<u32>(1)?];
            for b in &blocks {
                *if b.made { &mut made } else { &mut reused } += b.byte_length() as u64;
            }
            let [state, stats] = blocks;
            (Held::Block(state), Held::Block(stats))
        }
    };
    phases[0] += host_seconds() - begun;
    let at = host_seconds();
    let width = u64::from(device.simd_width()) * 4;
    let args = Args { nodes: nodes as u32, cells: cells as u32, pad: [0; 2] };
    let mut command = lane::Command::new("Rerail nearest");
    command.launch(&kernel, "nearest").buffer("state", state.buffer()).buffer("stats", stats.buffer()).value("args", &args).threads(cells as u64, width);
    phases[2] += host_seconds() - at;
    let times = command.run()?;
    let woke = times.committed + times.wall;
    let [kernel_at, start, end] = times.host;
    if end > 0.0 {
        phases[3] += (kernel_at - times.committed).max(0.0);
        phases[4] += (start - kernel_at).max(0.0);
        phases[6] += (woke - end).max(0.0);
    }
    phases[5] += times.device;
    let out = state.read(cells);
    if out.iter().any(|&v| v & FINAL == 0) {
        return Err(Error::BackendFailure);
    }
    let targets = out.into_iter().map(|v| if v == NONE { NONE } else { v & !FINAL }).collect();
    Ok(Solved { targets, rounds: stats.read(1)[0], launches: 1, gpu: times.device, wall: host_seconds() - begun, phases, made, reused })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every cell's nearest ancestor in its set, on the process's device, equals one pass in preorder on the host:
    /// a forest of a few thousand nodes, deep and shallow, two sets, in fresh buffers and in pooled blocks.
    #[test]
    fn the_device_finds_each_cells_nearest_ancestor_in_its_set() {
        if lane::device().is_err() {
            return;
        }
        // A preorder forest: a node's parent is the open node on top of a stack that grows and shrinks.
        let n = 5_000usize;
        let (mut parent, mut kind, mut stack) = (vec![NONE; n], vec![0u8; n], Vec::<u32>::new());
        let mut x = 0x2545_f491u32;
        for v in 0..n {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            if x % 7 == 0 || stack.len() > 200 {
                stack.clear();
            } else {
                for _ in 0..(x >> 8) % 3 {
                    stack.pop();
                }
            }
            parent[v] = stack.last().copied().unwrap_or(NONE);
            kind[v] = (x >> 16) as u8 % 5;
            stack.push(v as u32);
        }
        let sets: [&[u8]; 2] = [&[1, 3], &[4]];
        let (mut target, mut done, mut want) = (Vec::new(), Vec::new(), Vec::new());
        for set in sets {
            let mut one = vec![NONE; n];
            for v in 0..n {
                let p = parent[v];
                if p != NONE {
                    one[v] = if set.contains(&kind[p as usize]) { p } else { one[p as usize] };
                }
                target.push(p);
                done.push(u8::from(p == NONE || set.contains(&kind[p as usize])));
            }
            want.extend(one);
        }
        for how in [Buffers::Fresh, Buffers::Blocks] {
            let solved = solve_with(n, &target, &done, how).unwrap();
            assert_eq!(solved.targets, want, "{how:?} on {}", lane::backend());
            assert!(solved.rounds > 0);
        }
    }
}
