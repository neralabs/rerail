//! File-local lattice FIX on the device (list step 17; 02 §"Document-local lattice strata", 03 §7,
//! I10's R0): `logic::native::lattice_fixpoint`'s graph (seed values, copy edges, binary
//! transfers) over the flat lattice, solved on the device work queue (`metal_work_queue`, `lattice.uglykl`), on the process's device (decision 0440 step C). An
//! item is a cell to evaluate; a cell whose value rose pushes its dependents, which any idle
//! worker takes up, across every threadgroup, until none is active. The device, not the host,
//! decides each further evaluation (03 §7's device-side enqueue).
//!
//! The CPU's `lattice_fixpoint` is the reference: a monotone system over a finite lattice has one
//! least fixpoint, whatever order its cells are evaluated in.
use super::*;
pub use crate::code_rule_dsl::logic::device::lattice::{Solved, System};
use rerail_metal::lane;

const SOURCE: &str = include_str!("lattice.uglykl");
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
unsafe impl lane::Pod for Args {}

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
    lane::device()?;
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
    let kernel = lane::kernel_fixed(SOURCE, "lattice_queue", None);
    let ran = super::work_queue::run(&kernel, &queue, groups, "Rerail lattice FIX", |launch| {
        let rows: [(&'static str, &super::blocks::Block); 14] = [
            ("state", &state),
            ("values", &values),
            ("seeds", &seeds),
            ("copyStart", &copy_start),
            ("copies", &copies),
            ("binStart", &bin_start),
            ("binIndex", &bin_index),
            ("bins", &bins),
            ("outStart", &out_start),
            ("outs", &outs),
            ("queued", &queued),
            ("slots", &queue.slots),
            ("control", &queue.control),
            ("stats", &stats),
        ];
        rows.into_iter().fold(launch, |l, (name, b)| l.buffer(name, b.device_buffer())).value("args", &args)
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

#[cfg(test)]
mod tests {
    use super::*;

    fn lub(a: i64, b: i64) -> i64 {
        if a == BOT {
            b
        } else if b == BOT || a == b {
            a
        } else {
            TOP
        }
    }

    fn transfer(op: u8, a: i64, b: i64) -> i64 {
        const BIG: i64 = 1 << 40;
        match op {
            _ if a == TOP || b == TOP => TOP,
            b'+' => a + b,
            b'-' => a - b,
            b'*' if a.abs() < BIG && b.abs() < BIG => a * b,
            _ => TOP,
        }
    }

    /// The least fixpoint by rounds over every cell until none changes: the reference.
    fn fixpoint(s: &System<'_>) -> Vec<i64> {
        let mut v = s.seeds.to_vec();
        loop {
            let mut next = s.seeds.to_vec();
            for &(cell, source) in s.copies {
                next[cell as usize] = lub(next[cell as usize], v[source as usize]);
            }
            for &(cell, op, l, r) in s.bins {
                let (a, b) = (v[l as usize], v[r as usize]);
                if a != BOT && b != BOT {
                    next[cell as usize] = lub(next[cell as usize], transfer(op, a, b));
                }
            }
            if next == v {
                return v;
            }
            v = next;
        }
    }

    /// The device's fixpoint, on the process's device, equals the host's: 20,000 cells, copies along chains and
    /// back (cycles), transfers of each operator, seeds of every kind.
    #[test]
    fn the_device_reaches_the_lattices_least_fixpoint() {
        if lane::device().is_err() {
            return;
        }
        let n = 20_000u32;
        let mut x = 0x9e37_79b9u32;
        let mut next = || {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            x
        };
        let seeds: Vec<i64> = (0..n).map(|_| match next() % 10 { 0 => 7, 1 => i64::from(next() % 5), 2 => TOP, _ => BOT }).collect();
        let mut copies = Vec::new();
        let mut bins = Vec::new();
        for c in 1..n {
            match next() % 4 {
                0 => copies.push((c, c - 1)),
                1 => copies.push((c, next() % n)),
                2 => bins.push((c, [b'+', b'-', b'*', b'/'][(next() % 4) as usize], next() % c, next() % c)),
                _ => {}
            }
        }
        let system = System { seeds: &seeds, copies: &copies, bins: &bins };
        let want = fixpoint(&system);
        let solved = solve(&system).unwrap();
        assert_eq!(solved.values, want, "on {}", lane::backend());
        assert!(solved.visits > 0);
    }
}
