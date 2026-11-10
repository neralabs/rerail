//! The measurements behind [`Prices::default`]: each (form, lane) that has a kernel, at a size where the fixed
//! costs are small next to the work, and each lane's fixed costs. They are meant for a release build of this crate
//! alone, one lane a run, each within two minutes:
//!
//! `cargo test --release -p rerail-accel price::bench::cpu -- --ignored --nocapture` (and `sme`, `gpu`, `ane`).
//!
//! What each lane's price is, per unit ([`Kernel::units`]):
//!
//! - the cores: CPU seconds (user + sys) of the reference on a pool of one thread, so core-seconds, with no other
//!   thread's spinning in them;
//! - SME: the wall time of a submission (its workers stream; the CPU time is theirs);
//! - the GPU: device time (`GPUEndTime - GPUStartTime`);
//! - the Neural Engine: the wall time of its prediction calls.
//!
//! Each is the best of several runs: the machine is shared, and the best run is the one least disturbed.

use super::*;
use crate::cpu::tests::{bits, graph, parts, Rng};
use crate::exec::process_cpu;
use crate::kernel::Bits;
use std::sync::Arc;
use std::time::Instant;

fn load() -> String {
    std::process::Command::new("uptime").output().map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string()).unwrap_or_default()
}

/// A graph whose condensation has about `layers` levels (the closure's spike shape, as the GPU's timing test).
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

/// The spike-sized workloads (`tools/matrix-offload` W1–W5), by name.
fn workloads() -> Vec<(&'static str, Kernel)> {
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
fn tiny(f: Form) -> Kernel {
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
fn best(n: usize, mut f: impl FnMut() -> f64) -> f64 {
    (0..n).map(|_| f()).fold(f64::INFINITY, f64::min)
}

fn row(name: &str, lane: &str, k: &Kernel, secs: f64, what: &str) {
    println!("| {name} | {lane} | {:.3e} | {:.3} ms {what} | {:.2e} |", k.units(), secs * 1e3, secs / k.units());
}

fn header() {
    println!("load before: {}", load());
    println!("| workload | lane | units | best | s / unit |");
    println!("|---|---|---|---|---|");
}

#[test]
#[ignore = "prices: release build, run alone with --ignored --nocapture"]
fn cpu() {
    header();
    let one = rayon::ThreadPoolBuilder::new().num_threads(1).build().unwrap();
    for (name, k) in workloads() {
        let want = crate::cpu::run(&k);
        let runs = if k.units() > 1e10 { 3 } else { 7 };
        let secs = one.install(|| {
            best(runs, || {
                let c0 = process_cpu();
                let out = crate::cpu::run(&k);
                let c = process_cpu() - c0;
                assert!(out == want);
                c
            })
        });
        row(name, "cpu (core-s, user+sys)", &k, secs, "CPU");
    }
    // A launch's fixed cost: one tiny launch of each form through the lane's submission (the rayon pool), wall.
    let ex = crate::exec::CpuBackend::default();
    use crate::exec::Backend;
    let tinies: Vec<Kernel> = Form::ALL.iter().map(|&f| tiny(f)).collect();
    let refs: Vec<&Kernel> = tinies.iter().collect();
    let w = best(200, || {
        let t = Instant::now();
        std::hint::black_box(ex.submit(&refs).unwrap());
        t.elapsed().as_secs_f64()
    });
    println!("fixed: a submission of {} tiny launches: wall {:.1} us, {:.1} us a launch", refs.len(), w * 1e6, w * 1e6 / refs.len() as f64);
    println!("load after: {}", load());
}

#[cfg(all(feature = "sme", target_os = "macos", target_arch = "aarch64"))]
#[test]
#[ignore = "prices: release build, run alone with --ignored --nocapture"]
fn sme() {
    header();
    let s = crate::sme::backend().expect("this machine has SME2");
    for (name, k) in workloads().into_iter().filter(|(_, k)| s.can(k)) {
        let want = crate::cpu::run(&k);
        let mut cpu = f64::INFINITY;
        let secs = best(7, || {
            let (c0, t) = (process_cpu(), Instant::now());
            let out = s.submit(&[&k]).unwrap();
            let w = t.elapsed().as_secs_f64();
            cpu = cpu.min(process_cpu() - c0);
            assert!(out[0] == want);
            w
        });
        row(name, "sme (wall)", &k, secs, &format!("wall, {:.3} ms CPU", cpu * 1e3));
    }
    let k = tiny(Form::BitProduct);
    let w = best(200, || {
        let t = Instant::now();
        std::hint::black_box(s.submit(&[&k]).unwrap());
        t.elapsed().as_secs_f64()
    });
    let ks: Vec<Kernel> = (0..16).map(|_| k.clone()).collect();
    let refs: Vec<&Kernel> = ks.iter().collect();
    let w16 = best(200, || {
        let t = Instant::now();
        std::hint::black_box(s.submit(&refs).unwrap());
        t.elapsed().as_secs_f64()
    });
    println!("fixed: a submission of one tiny product: wall {:.1} us; of 16: {:.1} us ({:.2} us a launch more)", w * 1e6, w16 * 1e6, (w16 - w) * 1e6 / 15.0);
    println!("load after: {}", load());
}

#[cfg(all(feature = "gpu", target_os = "macos"))]
#[test]
#[ignore = "prices: release build, run alone with --ignored --nocapture"]
fn gpu() {
    use crate::exec::Backend;
    header();
    let g = crate::gpu::gpu_backend().expect("this machine has a Metal device");
    for (name, k) in workloads().into_iter().filter(|(_, k)| g.can(k)) {
        let want = crate::cpu::run(&k);
        let (out, _) = g.run(&[&k]).unwrap();
        assert!(out[0] == want, "{name}");
        let secs = best(7, || g.run(&[&k]).unwrap().1.device);
        row(name, "gpu (device)", &k, secs, "device");
    }
    // A submission's fixed cost: one tiny launch, wall and host CPU (the device's own time is a few µs).
    let k = tiny(Form::Histogram);
    let (mut cpu, mut dev) = (f64::INFINITY, f64::INFINITY);
    let w = best(50, || {
        let (c0, t) = (process_cpu(), Instant::now());
        let (_, s) = g.run(&[&k]).unwrap();
        let w = t.elapsed().as_secs_f64();
        cpu = cpu.min(process_cpu() - c0);
        dev = dev.min(s.device);
        w
    });
    let ks: Vec<Kernel> = Form::ALL.iter().cycle().take(20).map(|&f| tiny(f)).collect();
    let refs: Vec<&Kernel> = ks.iter().collect();
    let w20 = best(50, || {
        let t = Instant::now();
        std::hint::black_box(g.run(&refs).unwrap());
        t.elapsed().as_secs_f64()
    });
    println!(
        "fixed: a submission of one tiny launch: wall {:.1} us, host CPU {:.1} us, device {:.1} us; of 20: wall {:.1} us ({:.2} us a launch more)",
        w * 1e6,
        cpu * 1e6,
        dev * 1e6,
        w20 * 1e6,
        (w20 - w) * 1e6 / 19.0
    );
    println!("load after: {}", load());
}

#[cfg(all(feature = "ane", target_os = "macos"))]
#[test]
#[ignore = "prices: release build, run alone with --ignored --nocapture"]
fn ane() {
    use crate::ane::{Ane, MemoryStore};
    use crate::exec::Backend;
    header();
    let lane = Ane::new(Arc::new(MemoryStore::default()));
    let mut ws = workloads();
    ws.retain(|(_, k)| matches!(k, Kernel::BitProduct { .. }));
    for (name, k) in &ws {
        assert_eq!(lane.refusal(k), None, "{name}");
        let want = crate::cpu::run(k);
        assert!(lane.submit(&[k]).unwrap()[0] == want, "{name}");
        let mut split = Vec::new();
        let (mut cpu, mut wall) = (f64::INFINITY, f64::INFINITY);
        for _ in 0..10 {
            let (c0, t) = (process_cpu(), Instant::now());
            std::hint::black_box(lane.submit(&[k]).unwrap());
            wall = wall.min(t.elapsed().as_secs_f64());
            cpu = cpu.min(process_cpu() - c0);
            split.push(lane.last_split());
        }
        let predict = split.iter().map(|s| s.predict).fold(f64::INFINITY, f64::min);
        let host = split.iter().map(|s| s.expand + s.sum).fold(f64::INFINITY, f64::min);
        let s = split[0];
        row(name, "ane (prediction wall)", k, predict, &format!("in {} calls", s.calls));
        println!(
            "  {name}: {} tiles; a submission's best wall {:.3} ms, host CPU {:.3} ms (user+sys); host expand + sum {:.3} ms wall ({:.3} ms a tile)",
            s.tiles,
            wall * 1e3,
            cpu * 1e3,
            host * 1e3,
            host * 1e3 / s.tiles as f64
        );
    }
    // A tile's fixed cost: the W1 constant against one tile's rows and against one row (the same model).
    let Kernel::BitProduct { b, .. } = &ws[0].1 else { unreachable!() };
    let mut rng = Rng(9);
    for rows in [1usize, 2048] {
        let k = Kernel::BitProduct { a: bits(&mut rng, rows, 768, 1), b: b.clone(), min: None };
        let _ = lane.submit(&[&k]).unwrap();
        let (mut wall, mut predict) = (f64::INFINITY, f64::INFINITY);
        for _ in 0..20 {
            let t = Instant::now();
            std::hint::black_box(lane.submit(&[&k]).unwrap());
            wall = wall.min(t.elapsed().as_secs_f64());
            predict = predict.min(lane.last_split().predict);
        }
        println!("fixed: {rows} rows against W1's constant (one tile): submission wall {:.3} ms, prediction {:.3} ms", wall * 1e3, predict * 1e3);
    }
    println!("load after: {}", load());
}
