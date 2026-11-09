//! The gpu lane against the reference ([`crate::cpu::run`]), exactly, on random inputs of many shapes.

use super::*;
use crate::exec::{Batch, Executor, Launch, process_cpu};
use crate::kernel::Kernel;

// The reference's generators (crate::cpu::tests is private to the cpu module), the same xorshift and shapes.
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

fn parts(rng: &mut Rng, nparts: usize, max: u64, codes: u64) -> (Arc<[u32]>, Arc<[u16]>) {
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

fn graph(rng: &mut Rng, n: usize, deg: u64) -> (Arc<[u32]>, Arc<[u32]>) {
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

/// A graph whose condensation has about `layers` levels: edges mostly to lower layers, some within a layer.
fn layered(rng: &mut Rng, n: usize, deg: u64, layers: usize) -> (Arc<[u32]>, Arc<[u32]>) {
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

fn gpu() -> GpuBackend {
    gpu_backend().expect("this machine has a Metal device")
}

/// Every kernel through one submission, compared with the reference.
fn check(ks: &[Kernel]) -> Stats {
    let g = gpu();
    for (i, k) in ks.iter().enumerate() {
        assert!(g.can(k), "launch {i} ({}) refused", k.form().name());
    }
    let refs: Vec<&Kernel> = ks.iter().collect();
    let (outs, stats) = g.run(&refs).unwrap();
    for (i, (o, k)) in outs.iter().zip(ks).enumerate() {
        let want = crate::cpu::run(k);
        if *o != want {
            let brief = |o: &Output| match o {
                Output::Counts { rows, cols, data } => format!("counts {rows}x{cols}, sum {}", data.iter().map(|&x| x as u64).sum::<u64>()),
                Output::PerRow(v) => format!("per row {}, sum {}", v.len(), v.iter().map(|&x| x as u64).sum::<u64>()),
                Output::Pairs(p) => format!("{} pairs, first {:?}", p.len(), p.iter().take(4).collect::<Vec<_>>()),
                Output::Bits(b) => format!("bits {}x{}, ones {}", b.rows, b.words, b.data.iter().map(|w| w.count_ones()).sum::<u32>()),
            };
            panic!("launch {i} ({}): gpu {} vs reference {}", k.form().name(), brief(o), brief(&want));
        }
    }
    stats
}

#[test]
fn bit_product_counts_and_pairs() {
    let mut rng = Rng(17);
    let mut ks = Vec::new();
    for (n, m, width, da, db) in [(1, 1, 1, 8, 8), (37, 23, 200, 5, 3), (64, 64, 64, 4, 4), (65, 129, 513, 6, 2), (200, 7, 1000, 2, 9), (3, 300, 130, 8, 8), (10, 10, 0, 0, 0)] {
        let (a, b) = (bits(&mut rng, n, width, da), bits(&mut rng, m, width, db));
        ks.push(Kernel::BitProduct { a: a.clone(), b: b.clone(), min: None });
        let min: Vec<u32> = (0..m).map(|_| rng.below(width as u64 / 8 + 2) as u32).collect();
        ks.push(Kernel::BitProduct { a: a.clone(), b: b.clone(), min: Some(min.into()) });
        ks.push(Kernel::BitProduct { a, b, min: Some(vec![1; m].into()) });
    }
    let s = check(&ks);
    assert_eq!(s.command_buffers, 1);
}

#[test]
fn count_table_counts_and_pairs() {
    let mut rng = Rng(11);
    let mut ks = Vec::new();
    for (nparts, lmax, rmax, codes) in [(1, 1, 1, 1), (40, 30, 25, 12), (7, 700, 300, 40), (25, 60, 90, 2048), (25, 60, 90, 2049), (12, 300, 400, 65536), (30, 0, 20, 5), (30, 20, 0, 5), (0, 0, 0, 1)] {
        let (loff, left) = parts(&mut rng, nparts, lmax, codes);
        let (roff, right) = parts(&mut rng, nparts, rmax, codes);
        for emit in [false, true] {
            ks.push(Kernel::CountTable { loff: loff.clone(), left: left.clone(), roff: roff.clone(), right: right.clone(), emit });
        }
    }
    let s = check(&ks);
    assert_eq!(s.command_buffers, 1);
}

#[test]
fn histogram_counts() {
    let mut rng = Rng(3);
    let mut ks = Vec::new();
    for (nparts, max, width) in [(1, 1, 1), (30, 50, 9), (5, 3000, 128), (20, 400, 2048), (20, 400, 2049), (6, 2000, 5000), (3, 1000, 65536), (0, 0, 4), (10, 0, 7)] {
        let (off, codes) = parts(&mut rng, nparts, max, width as u64);
        ks.push(Kernel::Histogram { off, codes, width });
    }
    check(&ks);
    // A code at or above the width is an error, not a count.
    let k = Kernel::Histogram { off: vec![0, 2].into(), codes: vec![1, 9].into(), width: 4 };
    assert!(gpu().submit(&[&k]).unwrap_err().contains("not below the width"));
}

#[test]
fn closure_is_the_reference() {
    let mut rng = Rng(5);
    let mut ks = Vec::new();
    for (n, deg) in [(1, 1), (2, 1), (60, 1), (64, 1), (65, 2), (150, 2), (300, 1), (1000, 1), (700, 0), (129, 8)] {
        let (off, dst) = graph(&mut rng, n, deg);
        ks.push(Kernel::Closure { n: n as u32, off, dst });
    }
    for (n, deg, layers) in [(500, 2, 35), (3000, 3, 40), (200, 1, 200)] {
        let (off, dst) = layered(&mut rng, n, deg, layers);
        ks.push(Kernel::Closure { n: n as u32, off, dst });
    }
    // A chain: one level per node.
    let n = 300u32;
    ks.push(Kernel::Closure { n, off: (0..=n).map(|v| v.min(n - 1)).collect::<Vec<_>>().into(), dst: (1..n).collect::<Vec<_>>().into() });
    ks.push(Kernel::Closure { n: 0, off: vec![0].into(), dst: Vec::<u32>::new().into() });
    check(&ks);
}

#[test]
fn pairs_past_the_first_buffer_are_rerun_exactly() {
    let mut rng = Rng(23);
    // 2,100 x 2,100 pairs, every one kept: 4.4 M, past PAIRS_CAP.
    let (a, b) = (bits(&mut rng, 2100, 64, 4), bits(&mut rng, 2100, 64, 4));
    // One partition of 3,000 x 3,000 rows with one code: 9 M pairs.
    let ks = vec![
        Kernel::BitProduct { a, b, min: Some(vec![0; 2100].into()) },
        Kernel::CountTable { loff: vec![0, 3000].into(), left: vec![7; 3000].into(), roff: vec![0, 3000].into(), right: vec![7; 3000].into(), emit: true },
    ];
    let s = check(&ks);
    assert_eq!(s.command_buffers, 2);
}

#[test]
fn a_mixed_batch_is_one_command_buffer_through_the_executor() {
    let mut rng = Rng(9);
    let (off, codes) = parts(&mut rng, 50, 80, 30);
    let (loff, left) = parts(&mut rng, 50, 40, 30);
    let (goff, gdst) = graph(&mut rng, 400, 2);
    let (a, b) = (bits(&mut rng, 90, 300, 4), bits(&mut rng, 70, 300, 4));
    let ks = vec![
        Kernel::Histogram { off: off.clone(), codes: codes.clone(), width: 30 },
        Kernel::BitProduct { a: a.clone(), b: b.clone(), min: None },
        Kernel::BitProduct { a: a.clone(), b: b.clone(), min: Some(vec![9; 70].into()) },
        Kernel::CountTable { loff: loff.clone(), left: left.clone(), roff: off.clone(), right: codes.clone(), emit: false },
        Kernel::CountTable { loff, left, roff: off, right: codes, emit: true },
        Kernel::Closure { n: 400, off: goff, dst: gdst },
    ];
    let s = check(&ks);
    assert_eq!(s.command_buffers, 1);
    let ex = Executor::with(vec![backend().unwrap()]);
    let (done, report) = ex.run(Batch(ks.iter().cloned().map(Launch::new).collect())).unwrap();
    assert_eq!(report.launches[Lane::Gpu.index()], ks.len());
    for (d, k) in done.iter().zip(&ks) {
        assert_eq!(d.lane, Lane::Gpu);
        assert_eq!(d.out, crate::cpu::run(k));
    }
}

#[test]
fn page_aligned_inputs_are_not_copied() {
    let g = gpu();
    let page = g.gpu.page;
    let n = page / 4 * 3;
    // SAFETY: an anonymous mapping of three pages, unmapped at the end.
    let p = unsafe { libc::mmap(std::ptr::null_mut(), n * 4, libc::PROT_READ | libc::PROT_WRITE, libc::MAP_ANON | libc::MAP_PRIVATE, -1, 0) };
    assert_ne!(p, libc::MAP_FAILED);
    let col = unsafe { std::slice::from_raw_parts_mut(p as *mut u32, n) };
    col.iter_mut().enumerate().for_each(|(i, x)| *x = i as u32 * 7);
    let mut plan = Planner { gpu: g.gpu, inputs: HashMap::new() };
    let wrapped = plan.input(&col[..]);
    assert_eq!(wrapped.contents(), p, "wrapped in place");
    let copied = plan.input(&col[1..]);
    assert_ne!(copied.contents(), p.wrapping_add(4), "an unaligned column is copied");
    assert_eq!(read::<u32>(&copied, n - 1), col[1..]);
    // The same column twice is one buffer.
    assert_eq!(plan.input(&col[..]).contents(), p);
    drop((wrapped, copied, plan));
    unsafe { libc::munmap(p, n * 4) };
}

#[test]
fn can_is_honest_about_shapes() {
    let g = gpu();
    let mut rng = Rng(1);
    let (a, b) = (bits(&mut rng, 4, 64, 4), bits(&mut rng, 4, 128, 4));
    assert!(!g.can(&Kernel::BitProduct { a: a.clone(), b, min: None }), "different widths");
    assert!(!g.can(&Kernel::BitProduct { a: a.clone(), b: a.clone(), min: Some(vec![1; 3].into()) }), "a threshold missing");
    let k = Kernel::CountTable { loff: vec![0, 1].into(), left: vec![1].into(), roff: vec![0, 0, 1].into(), right: vec![1].into(), emit: false };
    assert!(!g.can(&k), "different partitions");
    // 70,000 x 70,000 pairs in one partition: more than u32 offsets.
    let big = Kernel::CountTable { loff: vec![0, 70_000].into(), left: vec![0u16; 70_000].into(), roff: vec![0, 70_000].into(), right: vec![0u16; 70_000].into(), emit: true };
    assert!(!g.can(&big));
    assert!(!g.can(&Kernel::Closure { n: 3, off: vec![0, 0].into(), dst: Vec::<u32>::new().into() }));
}

/// Spike-sized inputs (the shapes of `tools/matrix-offload` W1–W4), each alone and all in one submission: device
/// time, host CPU time and the price per unit. Run with `--ignored --nocapture`.
#[test]
#[ignore]
fn timing_on_spike_sized_inputs() {
    let g = gpu();
    let mut rng = Rng(42);
    // W1: 13,088 documents' name bits against 444 rules, 12 words a row.
    let w1 = Kernel::BitProduct { a: bits(&mut rng, 13088, 768, 1), b: bits(&mut rng, 444, 768, 1), min: None };
    let w1p = Kernel::BitProduct { a: bits(&mut rng, 13088, 768, 1), b: bits(&mut rng, 444, 768, 1), min: Some((0..444).map(|j| 3 + j % 5).collect::<Vec<u32>>().into()) };
    // W2: 13,088 documents, 5.8 M reads + bindings, local ids below 2,764.
    let (loff, left) = parts(&mut rng, 13088, 540, 2764);
    let (roff, right) = parts(&mut rng, 13088, 350, 2764);
    let w2 = Kernel::CountTable { loff: loff.clone(), left: left.clone(), roff: roff.clone(), right: right.clone(), emit: false };
    let w2e = Kernel::CountTable { loff, left, roff, right, emit: true };
    // W4: 22.3 M kinds, 13,088 documents, below 128.
    let (hoff, codes) = parts(&mut rng, 13088, 3400, 128);
    let w4 = Kernel::Histogram { off: hoff, codes, width: 128 };
    // W3: 12,000 nodes, 192 words a row, ~110 k edges, about 35 levels.
    let (coff, cdst) = layered(&mut rng, 12000, 5, 35);
    let w3 = Kernel::Closure { n: 12000, off: coff, dst: cdst };
    let all = [w1.clone(), w1p.clone(), w2.clone(), w2e.clone(), w4.clone(), w3.clone()];
    check(&all);
    let tiny = Kernel::Histogram { off: vec![0, 1].into(), codes: vec![0].into(), width: 1 };
    let time = |ks: &[&Kernel]| -> (f64, f64, f64, Stats) {
        // A warm-up, then the best of 7.
        let _ = g.run(ks).unwrap();
        let mut best = (f64::INFINITY, f64::INFINITY, f64::INFINITY, Stats::default());
        for _ in 0..7 {
            let (c0, t0) = (process_cpu(), std::time::Instant::now());
            let (_, s) = g.run(ks).unwrap();
            let (cpu, wall) = (process_cpu() - c0, t0.elapsed().as_secs_f64());
            if s.device < best.0 {
                best = (s.device, cpu, wall, s);
            }
        }
        best
    };
    let load = std::process::Command::new("uptime").output().map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string()).unwrap_or_default();
    println!("load: {load}");
    println!("| launch | units | device ms | host CPU ms | wall ms | s / unit | dispatches | barriers | CBs |");
    println!("|---|---|---|---|---|---|---|---|---|");
    let names = ["W1 bit product, counts", "W1 bit product, thresholded pairs", "W2 count table, counts", "W2 count table, pairs", "W4 histogram", "W3 closure"];
    let mut units = 0.0;
    for (name, k) in names.iter().zip(&all) {
        let (dev, cpu, wall, s) = time(&[k]);
        units += k.units();
        println!("| {name} | {:.3e} | {:.3} | {:.3} | {:.3} | {:.3e} | {} | {} | {} |", k.units(), dev * 1e3, cpu * 1e3, wall * 1e3, dev / k.units(), s.dispatches, s.barriers, s.command_buffers);
    }
    let refs: Vec<&Kernel> = all.iter().collect();
    let (dev, cpu, wall, s) = time(&refs);
    println!("| all six, one submission | {units:.3e} | {:.3} | {:.3} | {:.3} | | {} | {} | {} |", dev * 1e3, cpu * 1e3, wall * 1e3, s.dispatches, s.barriers, s.command_buffers);
    let (dev, cpu, wall, s) = time(&[&tiny]);
    println!("| fixed: one tiny histogram | 1 | {:.4} | {:.3} | {:.3} | | {} | {} | {} |", dev * 1e3, cpu * 1e3, wall * 1e3, s.dispatches, s.barriers, s.command_buffers);
    if let Output::Pairs(p) = crate::cpu::run(&w2e) {
        println!("W2 pairs: {}", p.len());
    }
}

/// Submissions from several threads at once, each of many closures: the planner's own host arrays (components,
/// levels) are freed and their addresses reused between launches, and must never be taken for another's.
#[test]
fn concurrent_submissions_are_exact() {
    let g = gpu();
    let mut rng = Rng(5);
    let mut ks: Vec<Kernel> = Vec::new();
    for (n, deg) in [(2, 1), (60, 1), (150, 2), (300, 1), (129, 8), (150, 2), (2, 1)] {
        let (off, dst) = graph(&mut rng, n, deg);
        ks.push(Kernel::Closure { n: n as u32, off, dst });
    }
    let (off, dst) = layered(&mut rng, 500, 2, 35);
    ks.push(Kernel::Closure { n: 500, off, dst });
    let (off, codes) = parts(&mut rng, 40, 60, 20);
    ks.push(Kernel::Histogram { off: off.clone(), codes: codes.clone(), width: 20 });
    ks.push(Kernel::CountTable { loff: off.clone(), left: codes.clone(), roff: off, right: codes, emit: true });
    let want: Vec<Output> = ks.iter().map(crate::cpu::run).collect();
    let refs: Vec<&Kernel> = ks.iter().collect();
    std::thread::scope(|sc| {
        for _ in 0..4 {
            sc.spawn(|| {
                for _ in 0..10 {
                    let (o, _) = g.run(&refs).unwrap();
                    for (i, (o, w)) in o.iter().zip(&want).enumerate() {
                        assert_eq!(o, w, "launch {i}");
                    }
                }
            });
        }
    });
}
