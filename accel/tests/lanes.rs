//! One wave across every lane: the executor places a batch's launches, submits each lane's share once, runs the
//! lanes together, and every output is the reference's.

use rerail_accel::{Batch, Bits, Executor, Kernel, Lane, Launch, cpu};
use std::sync::Arc;

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

fn bits(rng: &mut Rng, rows: usize, width: usize, density: u64) -> Bits {
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

fn parts(rng: &mut Rng, n: usize, max: u64, codes: u64) -> (Arc<[u32]>, Arc<[u16]>) {
    let (mut off, mut col) = (vec![0u32], Vec::new());
    for _ in 0..n {
        for _ in 0..rng.below(max + 1) {
            col.push(rng.below(codes) as u16);
        }
        off.push(col.len() as u32);
    }
    (off.into(), col.into())
}

fn wave(rng: &mut Rng) -> Vec<(Kernel, Lane)> {
    let (loff, left) = parts(rng, 50, 40, 300);
    let (roff, right) = parts(rng, 50, 30, 300);
    let (hoff, codes) = parts(rng, 80, 200, 115);
    let (mut off, mut dst) = (vec![0u32], Vec::new());
    for _ in 0..500 {
        for _ in 0..rng.below(3) {
            dst.push(rng.below(500) as u32);
        }
        off.push(dst.len() as u32);
    }
    // Large enough for the Neural Engine (Core ML keeps smaller products on the CPU), the constant fixed.
    let (a, b) = (bits(rng, 2048, 4096, 2), bits(rng, 256, 4096, 2).fixed());
    let min: Arc<[u32]> = (0..256).map(|j| 30 + (j % 5) as u32).collect();
    vec![
        (Kernel::BitProduct { a: a.clone(), b: b.clone(), min: None }, Lane::Ane),
        (Kernel::BitProduct { a: a.clone(), b: b.clone(), min: Some(min) }, Lane::Sme),
        (Kernel::CountTable { loff: loff.clone(), left: left.clone(), roff: roff.clone(), right: right.clone(), emit: false }, Lane::Gpu),
        (Kernel::CountTable { loff, left, roff, right, emit: true }, Lane::Cpu),
        (Kernel::Histogram { off: hoff, codes, width: 115 }, Lane::Gpu),
        (Kernel::Closure { n: 500, off: off.into(), dst: dst.into() }, Lane::Cpu),
    ]
}

#[test]
fn a_wave_runs_on_every_lane_at_once_and_matches_the_reference() {
    let ex = Executor::new();
    let lanes = ex.lanes();
    let mut rng = Rng(42);
    let ks = wave(&mut rng);
    // Pinned: every present lane gets work in the same batch; a lane this machine lacks is pinned to the CPU.
    let batch = Batch(ks.iter().map(|(k, l)| Launch::on(k.clone(), if lanes.contains(l) { *l } else { Lane::Cpu })).collect());
    let (done, report) = ex.run(batch).unwrap();
    for ((k, _), d) in ks.iter().zip(&done) {
        assert_eq!(d.out, cpu::run(k), "{} on {}", k.form().name(), d.lane.name());
    }
    for l in &lanes {
        assert!(report.launches[l.index()] > 0, "no launch on {}", l.name());
    }
    eprintln!("pinned: {report:?}");
    // Placed by price: the same outputs, wherever they ran.
    let (done, report) = ex.run(Batch(ks.iter().map(|(k, _)| Launch::new(k.clone())).collect())).unwrap();
    for ((k, _), d) in ks.iter().zip(&done) {
        assert_eq!(d.out, cpu::run(k), "{} on {}", k.form().name(), d.lane.name());
    }
    eprintln!("placed: {:?}; {report:?}", done.iter().map(|d| (d.lane.name(), d.out.clone()).0).collect::<Vec<_>>());
}
