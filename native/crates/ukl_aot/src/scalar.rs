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
