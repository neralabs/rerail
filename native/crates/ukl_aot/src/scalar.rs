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

/// `wide_step` (`wide.uglykl`): a value, or a flag at bit 32 and a value, by early returns.
fn wide_step(a: u64, c: u32) -> u64 {
    if a & 1 == 0 {
        return u64::from(c);
    }
    let s = (a >> 2) as u32;
    if s.wrapping_sub(1) >= c {
        return 0xFFFF_FFFF;
    }
    if a & 2 != 0 {
        return u64::from(c.wrapping_sub(s));
    }
    1u64 << 32 | u64::from(c.wrapping_sub(s))
}

/// The 8 bytes of `a` from byte `at` up, as `ukl_ld_window` reads them.
fn window(a: &[u64], at: usize) -> u64 {
    let (w, s) = (at / 8, (at % 8) * 8);
    if s == 0 { a[w] } else { a[w] >> s | a[w + 1] << (64 - s) }
}

/// `wide_lanes` (`wide.uglykl`): thread `i`'s rows of answers, `a`, `b` and `c` every thread's, `k` the uniform value.
pub fn wide(a: &[u64], b: &[u64], c: &[u32], k: u64, i: usize) -> [u64; crate::WIDE_ROWS] {
    let (x, y, z) = (a[i], b[i], c[i]);
    let sh = z & 63;
    let r = if x & 1 != 0 { x ^ y } else { x.wrapping_add(u64::from(z)) };
    let mut acc = x;
    for _ in 0..z & 7 {
        acc = acc.wrapping_mul(3).wrapping_add(y);
    }
    let (sx, sy) = (x as i64, y as i64);
    [
        x.wrapping_add(y),
        x.wrapping_sub(y),
        x << sh,
        x >> sh,
        (sx >> sh) as u64,
        x << 13 | y >> 51,
        x << 40,
        x >> 35,
        (sx >> 40) as u64,
        x.wrapping_mul(y),
        if y != 0 { x / y } else { 7 },
        u64::from(x < y) | u64::from(x <= y) << 1 | u64::from(x == y) << 2 | u64::from(x != y) << 3 | u64::from(sx < sy) << 4 | u64::from(sx >= sy) << 5 | u64::from(x > y) << 6,
        u64::from(z).wrapping_add(x),
        z as i32 as i64 as u64,
        u64::from((x >> 3) as u32),
        r,
        u64::from(x.count_ones()) + (u64::from(y.leading_zeros()) << 8),
        a[(z as usize) % a.len()],
        wide_step(x, z),
        acc,
        x.wrapping_add(k).wrapping_add(k << 3),
        if x & 0xFFFF_FFFF == y & 0xFFFF_FFFF { 1 } else { 2 },
        if x != 0 { 5 } else { 6 },
        ((u128::from(x) * u128::from(y)) >> 64) as u64,
        window(a, i as usize * if k & 1 != 0 { 8 } else { 4 }),
        { let j = i as u32 + 3; u64::from(j >> 8) * 1000 + u64::from(j & 255) },
    ]
}
