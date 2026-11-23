//! The kernels' test inputs (the tests' and benches' of every lane, this crate's and rerail-accel-gpu's): the
//! reference's generators (a xorshift, bit rows, partitions, graphs), the spike-sized workloads (`tools/matrix-offload`
//! W1–W5) and each form's smallest launch, and the benches' table.

use crate::kernel::{Bits, Form, Kernel};
use std::sync::Arc;

/// A small deterministic generator (xorshift), so the tests need no crate.
pub struct Rng(pub u64);
impl Rng {
    pub fn next_u64(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    pub fn below(&mut self, n: u64) -> u64 {
        self.next_u64() % n.max(1)
    }
}

/// Random bit rows, `density` out of 16 bits set.
pub fn bits(rng: &mut Rng, rows: usize, width: usize, density: u64) -> Bits {
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
pub fn parts(rng: &mut Rng, nparts: usize, max: u64, codes: u64) -> (Arc<[u32]>, Arc<[u16]>) {
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
pub fn graph(rng: &mut Rng, n: usize, deg: u64) -> (Arc<[u32]>, Arc<[u32]>) {
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

pub fn load() -> String {
    std::process::Command::new("uptime").output().map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string()).unwrap_or_default()
}

/// A graph whose condensation has about `layers` levels (the closure's spike shape, as the GPU's timing test).
pub fn layered(rng: &mut Rng, n: usize, deg: u64, layers: usize) -> (Arc<[u32]>, Arc<[u32]>) {
    let layer = |v: usize| v * layers / n;
    let mut off = vec![0u32];
    let mut dst = Vec::new();
    for v in 0..n {
        let lo = (0..v).find(|&w| layer(w) == layer(v)).unwrap_or(v);
        for _ in 0..rng.below(2 * deg + 1) {
            let w = if rng.below(50) == 0 { lo + rng.below((v - lo + 1) as u64) as usize } else { rng.below(lo.max(1) as u64) as usize };
            dst.push(w as u32);
        }
        off.push(dst.len() as u32);
    }
    (off.into(), dst.into())
}

/// The spike-sized workloads (`tools/matrix-offload` W1–W5), by name.
pub fn workloads() -> Vec<(&'static str, Kernel)> {
    let mut rng = Rng(42);
    // W1: 13,088 documents' name bits against 444 rules, 12 words a row.
    let w1 = Kernel::BitProduct { a: bits(&mut rng, 13088, 768, 1), b: bits(&mut rng, 444, 768, 1).fixed(), min: None };
    // W5: 2,048 names against 2,048 names over 13,088 document bits.
    let b5 = bits(&mut rng, 2048, 13088, 5).fixed();
    let w5 = Kernel::BitProduct { a: Bits { fixed: None, ..b5.clone() }, b: b5, min: None };
    // W2: 13,088 documents, 5.8 M reads and bindings, local ids below 2,764.
    let (loff, left) = parts(&mut rng, 13088, 540, 2764);
    let (roff, right) = parts(&mut rng, 13088, 350, 2764);
    let w2 = Kernel::CountTable { loff: loff.clone(), left: left.clone(), roff: roff.clone(), right: right.clone(), emit: false };
    let w2e = Kernel::CountTable { loff, left, roff, right, emit: true };
    // W4: 22.3 M kinds in 13,088 documents, below 128.
    let (hoff, codes) = parts(&mut rng, 13088, 3400, 128);
    let w4 = Kernel::Histogram { off: hoff, codes, width: 128 };
    // W3: 12,000 nodes, about 35 levels.
    let (coff, cdst) = layered(&mut rng, 12000, 5, 35);
    let w3 = Kernel::Closure { n: 12000, off: coff, dst: cdst };
    vec![("W1 bit product", w1), ("W5 bit product", w5), ("W2 count table", w2), ("W2 count table, pairs", w2e), ("W4 histogram", w4), ("W3 closure", w3)]
}

/// The smallest launch of each form: what a launch costs beyond its work.
pub fn tiny(f: Form) -> Kernel {
    let mut rng = Rng(7);
    match f {
        Form::Program => unreachable!("a program prices itself"),
        Form::BitProduct => Kernel::BitProduct { a: bits(&mut rng, 4, 64, 4), b: bits(&mut rng, 4, 64, 4), min: None },
        Form::CountTable | Form::CountPairs => {
            Kernel::CountTable { loff: vec![0, 2].into(), left: vec![1, 2].into(), roff: vec![0, 2].into(), right: vec![2, 1].into(), emit: f == Form::CountPairs }
        }
        Form::Histogram => Kernel::Histogram { off: vec![0, 2].into(), codes: vec![0, 1].into(), width: 2 },
        Form::Closure => {
            let (off, dst) = graph(&mut rng, 8, 1);
            Kernel::Closure { n: 8, off, dst }
        }
        Form::Probe | Form::Tile => {
            let keys: crate::kernel::Keys = vec![crate::col::Col::from(vec![1i64, 2])].into();
            let by = if f == Form::Tile { crate::kernel::JoinBy::Tile } else { crate::kernel::JoinBy::Probe };
            Kernel::Join { left: keys.clone(), right: keys, right_id: None, by }
        }
    }
}

/// The best of `n` runs of `f`, which returns the seconds it measured.
pub fn best(n: usize, mut f: impl FnMut() -> f64) -> f64 {
    (0..n).map(|_| f()).fold(f64::INFINITY, f64::min)
}

pub fn row(name: &str, lane: &str, k: &Kernel, secs: f64, what: &str) {
    println!("| {name} | {lane} | {:.3e} | {:.3} ms {what} | {:.2e} |", k.units(), secs * 1e3, secs / k.units());
}

pub fn header() {
    println!("load before: {}", load());
    println!("| workload | lane | units | best | s / unit |");
    println!("|---|---|---|---|---|");
}
