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
use crate::workloads::*;
use crate::exec::process_cpu;
use std::time::Instant;

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

#[cfg(sme_lane)]
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


#[cfg(all(feature = "ane", target_os = "macos"))]
#[test]
#[ignore = "prices: release build, run alone with --ignored --nocapture"]
fn ane() {
    use crate::ane::{Ane, MemoryStore};
    use crate::exec::Backend;
    header();
    let lane = Ane::new(std::sync::Arc::new(MemoryStore::default()));
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

/// The Neural Engine's tiles with and without the overlap of the host's work and the predictions: per phase, a
/// submission's median (wall, and the lane thread's CPU for the host's phases), and its best wall and process CPU
/// (user + sys). Run alone, release: `cargo test --release -p rerail-accel price::bench::ane_overlap -- --ignored
/// --nocapture`.
#[cfg(all(feature = "ane", target_os = "macos"))]
#[test]
#[ignore = "timing: release build, run alone with --ignored --nocapture"]
fn ane_overlap() {
    use crate::ane::{Ane, MemoryStore, Split};
    use crate::exec::Backend;
    println!("load before: {}", load());
    let lane = Ane::new(std::sync::Arc::new(MemoryStore::default()));
    let mut rng = Rng(42);
    // W1 (7 tiles) and W5's constant against 8,192 names (4 tiles of 2,048 rows over 13,088 bits).
    let w1 = Kernel::BitProduct { a: bits(&mut rng, 13088, 768, 1), b: bits(&mut rng, 444, 768, 1).fixed(), min: None };
    let b5 = bits(&mut rng, 2048, 13088, 5).fixed();
    let w5 = Kernel::BitProduct { a: bits(&mut rng, 8192, 13088, 5), b: b5, min: None };
    println!("| workload | overlap | tiles | encode wall (CPU) | predict wall | decode wall (CPU) | host wait | Core ML CPU | submit wall | submit CPU (user+sys) |");
    println!("|---|---|---|---|---|---|---|---|---|---|");
    for (name, k, iters) in [("W1", &w1, 30), ("W5 x4", &w5, 8)] {
        assert_eq!(lane.refusal(k), None, "{name}");
        let want = crate::cpu::run(k);
        for on in [false, true, false, true] {
            lane.set_overlap(on);
            assert!(lane.submit(&[k]).unwrap()[0] == want, "{name}");
            let (mut splits, mut walls, mut cpus) = (Vec::<Split>::new(), Vec::new(), Vec::new());
            for _ in 0..iters {
                let (c0, t) = (process_cpu(), Instant::now());
                std::hint::black_box(lane.submit(&[k]).unwrap());
                walls.push(t.elapsed().as_secs_f64());
                cpus.push(process_cpu() - c0);
                splits.push(lane.last_split());
            }
            let med = |f: &dyn Fn(&Split) -> f64| {
                let mut v: Vec<f64> = splits.iter().map(f).collect();
                v.sort_by(f64::total_cmp);
                v[v.len() / 2] * 1e3
            };
            let s = splits[0];
            assert_eq!(s.overlap, on);
            walls.sort_by(f64::total_cmp);
            cpus.sort_by(f64::total_cmp);
            println!(
                "| {name} | {} | {} | {:.2} ({:.2}) | {:.2} | {:.2} ({:.2}) | {:.2} | {:.2} | best {:.2} / median {:.2} | best {:.2} / median {:.2} |",
                if on { "on" } else { "off" },
                s.tiles,
                med(&|s| s.expand),
                med(&|s| s.expand_cpu),
                med(&|s| s.predict),
                med(&|s| s.sum),
                med(&|s| s.sum_cpu),
                med(&|s| s.wait),
                med(&|s| s.predict_cpu),
                walls[0] * 1e3,
                walls[walls.len() / 2] * 1e3,
                cpus[0] * 1e3,
                cpus[cpus.len() / 2] * 1e3,
            );
        }
    }
    println!("(ms a submission; phases are medians; encode/decode CPU is the lane thread's, Core ML CPU the rest of the process over the tiles)");
    println!("load after: {}", load());
}
