//! The reference kernels against their definitions, written the slow obvious way, on small random inputs.

use super::*;
use crate::exec::{Batch, Executor, Launch};
use crate::lanes::Lane;
use std::sync::Arc;

/// A small deterministic generator (xorshift), so the tests need no crate.
pub(crate) struct Rng(pub u64);
impl Rng {
    pub(crate) fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    pub(crate) fn below(&mut self, n: u64) -> u64 {
        self.next() % n.max(1)
    }
}

/// Random bit rows, `density` out of 16 bits set.
pub(crate) fn bits(rng: &mut Rng, rows: usize, width: usize, density: u64) -> Bits {
    let words = width.div_ceil(64);
    let mut data = vec![0u64; rows * words];
    for r in 0..rows {
        for c in 0..width {
            if rng.below(16) < density {
                data[r * words + c / 64] |= 1 << (c % 64);
            }
        }
    }
    Bits::new(rows, words, data)
}

/// Random partitions of codes below `codes`.
pub(crate) fn parts(rng: &mut Rng, nparts: usize, max: u64, codes: u64) -> (Arc<[u32]>, Arc<[u16]>) {
    let mut off = vec![0u32];
    let mut col = Vec::new();
    for _ in 0..nparts {
        for _ in 0..rng.below(max + 1) {
            col.push(rng.below(codes) as u16);
        }
        off.push(col.len() as u32);
    }
    (off.into(), col.into())
}

/// A random graph of `n` nodes, about `deg` edges each, with some self edges and cycles.
pub(crate) fn graph(rng: &mut Rng, n: usize, deg: u64) -> (Arc<[u32]>, Arc<[u32]>) {
    let mut off = vec![0u32];
    let mut dst = Vec::new();
    for _ in 0..n {
        for _ in 0..rng.below(2 * deg + 1) {
            dst.push(rng.below(n as u64) as u32);
        }
        off.push(dst.len() as u32);
    }
    (off.into(), dst.into())
}

#[test]
fn bit_product_counts_and_thresholds() {
    let mut rng = Rng(7);
    let (a, b) = (bits(&mut rng, 37, 200, 5), bits(&mut rng, 23, 200, 3));
    let Output::Counts { rows, cols, data } = bit_product(&a, &b, None) else { panic!() };
    assert_eq!((rows, cols), (37, 23));
    for i in 0..37 {
        for j in 0..23 {
            let want = (0..200).filter(|&c| a.get(i, c) && b.get(j, c)).count() as u32;
            assert_eq!(data[i * 23 + j], want);
        }
    }
    let min: Vec<u32> = (0..23).map(|j| (j % 7) as u32 + 5).collect();
    let Output::Pairs(p) = bit_product(&a, &b, Some(&min)) else { panic!() };
    let want: Vec<(u32, u32)> =
        (0..37).flat_map(|i| (0..23).map(move |j| (i, j))).filter(|&(i, j)| data[i * 23 + j] >= min[j]).map(|(i, j)| (i as u32, j as u32)).collect();
    assert_eq!(p, want);
}

#[test]
fn count_table_counts_and_pairs() {
    let mut rng = Rng(11);
    let (loff, left) = parts(&mut rng, 40, 30, 12);
    let (roff, right) = parts(&mut rng, 40, 25, 12);
    let Output::PerRow(c) = count_table(&loff, &left, &roff, &right, false) else { panic!() };
    let Output::Pairs(pairs) = count_table(&loff, &left, &roff, &right, true) else { panic!() };
    let mut want = Vec::new();
    for p in 0..40 {
        for li in loff[p]..loff[p + 1] {
            let n = (roff[p]..roff[p + 1]).filter(|&ri| right[ri as usize] == left[li as usize]).inspect(|&ri| want.push((li, ri))).count();
            assert_eq!(c[li as usize], n as u32);
        }
    }
    assert_eq!(pairs, want);
}

#[test]
fn histogram_counts() {
    let mut rng = Rng(3);
    let (off, codes) = parts(&mut rng, 30, 50, 9);
    let Output::Counts { rows, cols, data } = histogram(&off, &codes, 9) else { panic!() };
    assert_eq!((rows, cols), (30, 9));
    for p in 0..30 {
        for k in 0..9u16 {
            assert_eq!(data[p * 9 + k as usize], codes[off[p] as usize..off[p + 1] as usize].iter().filter(|&&c| c == k).count() as u32);
        }
    }
}

#[test]
fn closure_is_reachability() {
    let mut rng = Rng(5);
    for (n, deg) in [(1, 1), (60, 1), (150, 2), (300, 1)] {
        let (off, dst) = graph(&mut rng, n, deg);
        let got = closure(n as u32, &off, &dst);
        // Reachability by search from each node's successors.
        for s in 0..n {
            let mut seen = vec![false; n];
            let mut todo: Vec<u32> = dst[off[s] as usize..off[s + 1] as usize].to_vec();
            while let Some(v) = todo.pop() {
                if std::mem::replace(&mut seen[v as usize], true) {
                    continue;
                }
                todo.extend(&dst[off[v as usize] as usize..off[v as usize + 1] as usize]);
            }
            for t in 0..n {
                assert_eq!(got.get(s, t), seen[t], "n {n}: {s} -> {t}");
            }
        }
    }
}

#[test]
fn the_executor_batches_and_places_on_the_cpu() {
    let mut rng = Rng(9);
    let (off, codes) = parts(&mut rng, 20, 40, 7);
    let (a, b) = (bits(&mut rng, 10, 128, 4), bits(&mut rng, 10, 128, 4));
    let ks = vec![
        Kernel::Histogram { off: off.clone(), codes: codes.clone(), width: 7 },
        Kernel::BitProduct { a: a.clone(), b: b.clone(), min: None },
    ];
    let ex = Executor::with(vec![Arc::new(crate::exec::CpuBackend)]);
    let (done, report) = ex.run(Batch(ks.iter().cloned().map(Launch::new).collect())).unwrap();
    assert_eq!(report.launches[Lane::Cpu.index()], 2);
    for (d, k) in done.iter().zip(&ks) {
        assert_eq!(d.lane, Lane::Cpu);
        assert_eq!(d.out, run(k));
    }
    // A launch pinned to a lane that is not there is an error, not a fallback.
    let err = ex.run(Batch(vec![Launch::on(ks[0].clone(), Lane::Gpu)])).unwrap_err();
    assert!(err.contains("no lane can run launch 0 (histogram"), "{err}");
}

#[test]
fn the_parity_band_prefers_the_least_energy() {
    use crate::kernel::Form;
    use crate::price::{Fixed, Prices};
    // The policy, at prices set for the test: the GPU 10x SME, the Neural Engine 2x slower than SME.
    let mut p = Prices::default();
    p.fixed = [Fixed { submission: 0.0, launch: 0.0 }; 4];
    p.set(Form::BitProduct, Lane::Cpu, Some(1e-11));
    p.set(Form::BitProduct, Lane::Sme, Some(1e-13));
    p.set(Form::BitProduct, Lane::Gpu, Some(1e-14));
    p.set(Form::BitProduct, Lane::Ane, Some(2e-13));
    let mut rng = Rng(1);
    let k = Kernel::BitProduct { a: bits(&mut rng, 256, 1024, 4), b: bits(&mut rng, 256, 1024, 4), min: None };
    let all = [Lane::Cpu, Lane::Sme, Lane::Gpu, Lane::Ane];
    // An idle GPU finishes 20x before the Neural Engine: outside the band, so the GPU.
    assert_eq!(p.choose(&k, &all, &[0.0; 4]).unwrap().0, Lane::Gpu);
    // Without the GPU, the Neural Engine is within the band of SME (2x) and draws the least power.
    assert_eq!(p.choose(&k, &[Lane::Cpu, Lane::Sme, Lane::Ane], &[0.0; 4]).unwrap().0, Lane::Ane);
    // With the GPU busy (a wave queued on it), the Neural Engine takes the work.
    let busy = 10.0 * p.wall(&k, Lane::Ane).unwrap();
    assert_eq!(p.choose(&k, &all, &[0.0, 0.0, busy, 0.0]).unwrap().0, Lane::Ane);
    // With a band of 1, the first to finish wins.
    p.band = 1.0;
    assert_eq!(p.choose(&k, &[Lane::Cpu, Lane::Sme, Lane::Ane], &[0.0; 4]).unwrap().0, Lane::Sme);
}
