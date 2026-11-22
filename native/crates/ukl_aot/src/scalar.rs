//! The kernels one thread at a time: the reference the others are compared with.

use crate::FINAL;

/// `nearest_jump` (`nearest.uglykl`): each cell's state followed to its answer, its thread's after the one before it.
/// Returns the most steps a thread took.
pub fn nearest_jump(state: &mut [u32], nodes: u32, cells: u32) -> u32 {
    let mut most = 0;
    for cell in 0..cells {
        let mut v = state[cell as usize];
        let base = (cell / nodes) * nodes;
        let mut steps = 0u32;
        while v & FINAL == 0 && steps < nodes {
            v = state[(base + v) as usize];
            steps += 1;
            state[cell as usize] = v;
        }
        most = most.max(steps);
    }
    most
}

/// `mix_lanes` (`kernels/mix.uglykl`) of one value, `rounds` its uniform loop's.
pub fn mix(x: u32, rounds: u32) -> u32 {
    let mut y = if x & 1 != 0 { x.wrapping_mul(3).wrapping_add(1) } else { x >> 1 };
    if x < 0xF000_0000 {
        y = y.wrapping_add(x);
    }
    y = if x == 0 { 0xDEAD } else { y ^ 7 };
    let mut s = 0u32;
    for k in 0..x & 15 {
        s = s.wrapping_add(k.wrapping_mul(y));
    }
    for r in 0..rounds {
        s = s.wrapping_mul(31).wrapping_add(r);
    }
    y.wrapping_add(s)
}
