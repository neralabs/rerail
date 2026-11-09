//! The SME lane against the reference (`crate::cpu::run`), exactly, on random shapes.

use super::*;
use crate::exec::{Batch, Executor, Launch, process_cpu};

/// xorshift, as the reference's tests use (theirs is private to `cpu`).
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n.max(1)
    }
}

fn sme() -> Sme {
    match backend() {
        Some(_) => Sme { units: lanes::sme_units() },
        None => panic!("this machine has no SME2"),
    }
}

/// Random rows of `width` bits, each bit set with chance `1 / 2^sparsity`, built a word at a time.
fn rows(rng: &mut Rng, rows: usize, width: usize, sparsity: u32) -> Bits {
    let words = width.div_ceil(64);
    let mut data = vec![0u64; rows * words];
    for r in 0..rows {
        for w in 0..words {
            let mut x = !0u64;
            for _ in 0..sparsity {
                x &= rng.next();
            }
            if w == words - 1 && width % 64 != 0 {
                x &= (1u64 << (width % 64)) - 1;
            }
            data[r * words + w] = x;
        }
    }
    Bits::new(rows, words, data)
}

fn check(s: &Sme, ks: &[Kernel]) {
    let refs: Vec<&Kernel> = ks.iter().collect();
    let got = s.submit(&refs).expect("sme submit");
    assert_eq!(got.len(), ks.len());
    for (n, (k, g)) in ks.iter().zip(&got).enumerate() {
        let want = crate::cpu::run(k);
        if *g != want {
            let Kernel::BitProduct { a, b, min } = k else { unreachable!() };
            panic!("launch {n}: {}x{} rows of {} words, min {}: differs from the reference", a.rows, b.rows, a.words, min.is_some());
        }
    }
}

#[test]
fn counts_many_shapes() {
    let s = sme();
    let mut rng = Rng(0x5eed);
    let shapes = [(1, 1, 1), (31, 33, 2), (32, 32, 3), (33, 65, 5), (7, 100, 64), (100, 7, 130), (64, 96, 300), (45, 17, 257)];
    for &(n, m, words) in &shapes {
        let width = words * 64 - rng.below(64) as usize;
        let (a, b) = (rows(&mut rng, n, width, 1), rows(&mut rng, m, width, 2));
        check(&s, &[Kernel::BitProduct { a, b, min: None }]);
    }
    for _ in 0..12 {
        let (n, m, words) = (1 + rng.below(90) as usize, 1 + rng.below(90) as usize, 1 + rng.below(300) as usize);
        let width = words * 64 - rng.below(64) as usize;
        let (sa, sb) = (rng.below(3) as u32, rng.below(3) as u32);
        let (a, b) = (rows(&mut rng, n, width, sa), rows(&mut rng, m, width, sb));
        check(&s, &[Kernel::BitProduct { a, b, min: None }]);
    }
}

#[test]
fn thresholds_many_shapes() {
    let s = sme();
    let mut rng = Rng(0xfeed);
    for t in 0..16 {
        let (n, m, words) = (1 + rng.below(80) as usize, 1 + rng.below(80) as usize, 1 + rng.below(300) as usize);
        let width = words * 64 - rng.below(64) as usize;
        let sb = 1 + rng.below(2) as u32;
        let (a, b) = (rows(&mut rng, n, width, 1), rows(&mut rng, m, width, sb));
        // Thresholds around the expected count, with some zeros (every pair) and some unreachable.
        let mid = (width / 8).max(1) as u64;
        let min: Vec<u32> = (0..m)
            .map(|j| match (t + j) % 9 {
                0 => 0,
                1 => u32::MAX,
                2 => 1,
                _ => rng.below(2 * mid) as u32,
            })
            .collect();
        check(&s, &[Kernel::BitProduct { a, b, min: Some(min.into()) }]);
    }
    // The semi-join: every threshold 1, on sparse rows.
    let (a, b) = (rows(&mut rng, 37, 200, 4), rows(&mut rng, 23, 200, 4));
    check(&s, &[Kernel::BitProduct { a, b, min: Some(vec![1; 23].into()) }]);
}

#[test]
fn self_product_writes_both_triangles() {
    let s = sme();
    let mut rng = Rng(0xabc);
    for &(n, words) in &[(1, 1), (31, 4), (32, 205), (97, 205), (130, 17)] {
        let a = rows(&mut rng, n, words * 64, 1);
        // The same buffer on both sides: the symmetric path. And with thresholds: the full path.
        let min: Vec<u32> = (0..n).map(|j| (j % 40) as u32 * words as u32).collect();
        check(&s, &[Kernel::BitProduct { a: a.clone(), b: a.clone(), min: None }, Kernel::BitProduct { a: a.clone(), b: a, min: Some(min.into()) }]);
    }
}

#[test]
fn a_batch_is_one_submission() {
    let s = sme();
    let mut rng = Rng(0x77);
    let shared = rows(&mut rng, 70, 3000, 1);
    let mut ks = Vec::new();
    for t in 0..9 {
        let n = 1 + rng.below(100) as usize;
        let a = rows(&mut rng, n, 3000, 2);
        let min = (t % 2 == 0).then(|| (0..70).map(|j| (j % 30) as u32).collect::<Vec<u32>>().into());
        ks.push(Kernel::BitProduct { a, b: shared.clone(), min });
    }
    check(&s, &ks);
}

#[test]
fn degenerate_shapes() {
    let s = sme();
    let mut rng = Rng(3);
    let a = rows(&mut rng, 5, 70, 1);
    let ks = [
        Kernel::BitProduct { a: Bits::zeros(0, 2), b: a.clone(), min: None },
        Kernel::BitProduct { a: a.clone(), b: Bits::zeros(0, 2), min: Some(Vec::new().into()) },
        Kernel::BitProduct { a: Bits::zeros(3, 0), b: Bits::zeros(4, 0), min: None },
        Kernel::BitProduct { a: Bits::zeros(3, 0), b: Bits::zeros(4, 0), min: Some(vec![0, 1, 0, 2].into()) },
        Kernel::BitProduct { a: Bits::zeros(3, 2), b: a, min: Some(vec![0, 1, 0, 2, 0].into()) },
    ];
    check(&s, &ks);
    assert!(!s.can(&Kernel::BitProduct { a: Bits::zeros(1, 1), b: Bits::zeros(1, 2), min: None }));
    assert!(!s.can(&Kernel::Histogram { off: vec![0, 0].into(), codes: Vec::new().into(), width: 1 }));
}

#[test]
fn the_executor_places_on_sme() {
    let mut rng = Rng(11);
    let (a, b) = (rows(&mut rng, 50, 640, 1), rows(&mut rng, 40, 640, 1));
    let k = Kernel::BitProduct { a, b, min: None };
    let want = crate::cpu::run(&k);
    let ex = Executor::with(vec![backend().unwrap()]);
    let (done, report) = ex.run(Batch(vec![Launch::on(k, Lane::Sme)])).unwrap();
    assert_eq!(done[0].lane, Lane::Sme);
    assert_eq!(done[0].out, want);
    assert_eq!(report.launches[Lane::Sme.index()], 1);
}

/// W5's shape: 2048 × 2048 rows of 205 words, the counts written (a self-product, so the symmetric path), and the
/// same product with distinct buffers (the full path). Reports CPU time (user + sys) and wall, and the price.
#[test]
#[ignore = "timing: run alone with --ignored --nocapture"]
fn timing_w5_shape() {
    let s = sme();
    let mut rng = Rng(0x55);
    let a = rows(&mut rng, 2048, 205 * 64, 3);
    let b = Bits::new(2048, 205, a.data.to_vec());
    let units = Kernel::BitProduct { a: a.clone(), b: a.clone(), min: None }.units();
    let time = |ks: &[Kernel], label: &str| {
        let refs: Vec<&Kernel> = ks.iter().collect();
        let mut best = (f64::INFINITY, f64::INFINITY, 0i64);
        for _ in 0..7 {
            let (c0, t0) = (process_cpu(), std::time::Instant::now());
            let (_, ops) = s.run(&refs).unwrap();
            let (wall, cpu) = (t0.elapsed().as_secs_f64(), process_cpu() - c0);
            if wall < best.0 {
                best = (wall, cpu, ops);
            }
        }
        let (wall, cpu, ops) = best;
        println!(
            "{label}: wall {:.3} ms, CPU {:.3} ms (user+sys, {} workers), {:.1} M BMOPA issued, {:.2e} s per bit-op of the product (wall), {:.1} Tbit-op/s",
            wall * 1e3,
            cpu * 1e3,
            s.units,
            ops as f64 / 1e6,
            wall / units,
            units / wall / 1e12
        );
    };
    time(&[Kernel::BitProduct { a: a.clone(), b: a.clone(), min: None }], "W5 self-product, counts");
    time(&[Kernel::BitProduct { a: a.clone(), b: b.clone(), min: None }], "W5 full product, counts");
    // Thresholds around the mean count (205 at this density): about half the pairs, then a few.
    for m in [205, 260] {
        let min: Arc<[u32]> = vec![m; 2048].into();
        let k = Kernel::BitProduct { a: a.clone(), b: b.clone(), min: Some(min.clone()) };
        let Output::Pairs(p) = &s.run(&[&k]).unwrap().0[0] else { unreachable!() };
        time(&[Kernel::BitProduct { a: a.clone(), b: b.clone(), min: Some(min) }], &format!("W5 full product, pairs (min {m}: {} pairs)", p.len()));
    }
    // The packing alone (one operand, on the rayon pool), part of each submission above.
    let mut best = (f64::INFINITY, 0.0);
    for _ in 0..7 {
        let (c0, t0) = (process_cpu(), std::time::Instant::now());
        std::hint::black_box(pack(&a));
        let (wall, cpu) = (t0.elapsed().as_secs_f64(), process_cpu() - c0);
        if wall < best.0 {
            best = (wall, cpu);
        }
    }
    println!("packing one 2048 x 205-word operand: wall {:.3} ms, CPU {:.3} ms", best.0 * 1e3, best.1 * 1e3);
    // The fixed cost: a submission of one tiny product.
    let t = (rows(&mut rng, 4, 64, 1), rows(&mut rng, 4, 64, 1));
    let tiny = [Kernel::BitProduct { a: t.0, b: t.1, min: None }];
    let refs: Vec<&Kernel> = tiny.iter().collect();
    let mut best = (f64::INFINITY, 0.0);
    for _ in 0..50 {
        let (c0, t0) = (process_cpu(), std::time::Instant::now());
        s.run(&refs).unwrap();
        let (wall, cpu) = (t0.elapsed().as_secs_f64(), process_cpu() - c0);
        if wall < best.0 {
            best = (wall, cpu);
        }
    }
    println!("fixed cost per submission (4x4 rows, 1 word): wall {:.1} us, CPU {:.1} us", best.0 * 1e6, best.1 * 1e6);
    // Correctness of the timed shape, once.
    let refs = [Kernel::BitProduct { a: a.clone(), b: a, min: None }];
    check(&s, &refs);
}
